mod collector;
mod config;
mod downloader;
mod error;

use clap::Parser;
use error::{AppError, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "osu-collect")]
#[command(version = "0.1.0")]
#[command(about = "A CLI tool to download osu! beatmap collections from osu!collector", long_about = None)]
struct Cli {
    /// Collection URL or ID
    #[arg(short, long)]
    collection: String,

    /// Download directory
    #[arg(short, long)]
    directory: String,

    /// Mirror base URL
    #[arg(short, long)]
    mirror: Option<String>,

    /// Auto-overwrite existing files
    #[arg(short, long)]
    yes: bool,

    /// Skip existing files
    #[arg(long)]
    skip_existing: bool,
}

impl Cli {
    /// Validate CLI arguments
    fn validate(&self) -> Result<()> {
        if self.yes && self.skip_existing {
            return Err(AppError::other_static(
                "Cannot use both --yes and --skip-existing flags"
            ));
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = cli.validate() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    let config = config::load_config()
        .merge_with_cli(cli.mirror.clone(), cli.skip_existing);

    if let Err(e) = config.validate() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    if let Err(e) = run(cli, config).await {
        eprintln!("\n\x1b[31m✗ Error: {}\x1b[0m", e);
        std::process::exit(1);
    }
}

async fn run(cli: Cli, config: config::Config) -> Result<()> {
    println!("osu! collect v0.1.0\n");

    println!("Fetching collection...");
    let collection_id = collector::parse_collection_id(&cli.collection)?;

    let collection_client = collector::create_collection_client()?;
    let collection = collector::fetch_collection(&collection_client, collection_id).await?;

    collector::display_collection_info(&collection);

    let output_dir = downloader::validate_and_prepare_directory(&cli.directory).await?;
    println!("\nDownloading to: {}\n", output_dir.display());

    let download_client = downloader::create_download_client()?;

    let total_beatmaps = collection.beatmapsets.len();
    let pb = ProgressBar::new(total_beatmaps as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg}\n{bar:40.cyan/blue} {pos}/{len} ({percent}%)")
            .unwrap()
            .progress_chars("█▓░"),
    );

    let mut downloaded_count: u16 = 0;
    let mut skipped_count: u16 = 0;
    let mut failed_count: u16 = 0;
    let mut failed_downloads: Vec<(u32, String)> = Vec::new();
    let mut aborted = false;

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            shutdown_clone.store(true, Ordering::SeqCst);
        }
    });

    let mut msg_buffer = String::with_capacity(64);

    for (idx, beatmapset) in collection.beatmapsets.iter().enumerate() {
        if shutdown.load(Ordering::SeqCst) {
            aborted = true;
            pb.println("\n\x1b[33m⚠  Download process interrupted by user (Ctrl+C)\x1b[0m");
            break;
        }

        msg_buffer.clear();
        msg_buffer.push_str("Current: ");
        msg_buffer.push_str(&beatmapset.id.to_string());
        msg_buffer.push_str(".osz");
        pb.set_message(msg_buffer.clone());

        let result = downloader::download_beatmap(
            &download_client,
            beatmapset.id,
            &config.mirror.url,
            &output_dir,
            config.download.skip_existing || cli.skip_existing,
            cli.yes,
        ).await?;

        match result {
            downloader::DownloadResult::Success(filename) => {
                downloaded_count += 1;
                pb.println(format!("\x1b[32m✓\x1b[0m Downloaded: {}", filename));
            }
            downloader::DownloadResult::Skipped(filename) => {
                skipped_count += 1;
                pb.println(format!("\x1b[33m⚠\x1b[0m Skipped (existing): {}", filename));
            }
            downloader::DownloadResult::Failed(reason) => {
                failed_count += 1;
                failed_downloads.push((beatmapset.id, reason.to_string()));
                pb.println(format!("\x1b[31m✗\x1b[0m Error downloading {}: {}", beatmapset.id, reason));
            }
            downloader::DownloadResult::FailedOwned(reason) => {
                failed_count += 1;
                failed_downloads.push((beatmapset.id, reason.clone()));
                pb.println(format!("\x1b[31m✗\x1b[0m Error downloading {}: {}", beatmapset.id, reason));
            }
            downloader::DownloadResult::Aborted => {
                aborted = true;
                pb.println("\x1b[33m⚠  Download process aborted by user\x1b[0m");
                break;
            }
        }

        pb.set_position((idx + 1) as u64);
    }

    pb.finish_and_clear();

    println!("\n================================");
    println!("Summary:");
    println!("\x1b[32m✓\x1b[0m Downloaded: {}", downloaded_count);
    println!("\x1b[33m⚠\x1b[0m Skipped (existing): {}", skipped_count);
    println!("\x1b[31m✗\x1b[0m Failed: {}", failed_count);

    if !failed_downloads.is_empty() {
        println!("\nFailed downloads:");
        for (id, reason) in failed_downloads {
            println!("  - {} ({})", id, reason);
        }
    }

    println!();

    if aborted {
        println!("\x1b[33mDownload process was interrupted.\x1b[0m");
    } else if failed_count == 0 && skipped_count == 0 {
        println!("\x1b[32mDone! All beatmaps downloaded successfully.\x1b[0m");
    } else if failed_count == 0 {
        println!("\x1b[32mDone! All available beatmaps downloaded.\x1b[0m");
    } else {
        println!("\x1b[33mCompleted with errors.\x1b[0m");
    }

    Ok(())
}
