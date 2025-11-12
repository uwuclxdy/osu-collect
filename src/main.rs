mod collector;
mod collection;
mod config;
mod downloader;
mod error;
mod utils;

use clap::Parser;
use error::{AppError, Result};
use futures_util::stream::{self, StreamExt};
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
            return Err(AppError::other(
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
    let collection_id = utils::parse_collection_id(&cli.collection)?;

    let collection_client = collector::create_collection_client()?;
    let collection = collector::fetch_collection(&collection_client, collection_id).await?;

    collector::display_collection_info(&collection);

    let base_dir = downloader::validate_and_prepare_directory(&cli.directory).await?;

    let collection_folder_name = collection::generate_collection_folder_name(&collection);
    let output_dir = base_dir.join(&collection_folder_name);

    tokio::fs::create_dir_all(&output_dir).await?;

    println!("\nCollection folder: {}", collection_folder_name);
    println!("Downloading to: {}\n", output_dir.display());

    let download_client = downloader::create_download_client()?;

    let total_beatmaps = collection.beatmapsets.len();
    let pb = ProgressBar::new(total_beatmaps as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg}\n{bar:40.cyan/blue} {pos}/{len} ({percent}%)")
            .unwrap()
            .progress_chars("█▓░"),
    );

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            shutdown_clone.store(true, Ordering::SeqCst);
        }
    });

    let concurrent = config.download.concurrent as usize;
    let skip_existing = config.download.skip_existing || cli.skip_existing;

    let results = stream::iter(collection.beatmapsets.iter())
        .map(|beatmapset| {
            let client = download_client.clone();
            let mirror_url = config.mirror.url.to_string();
            let output_dir = output_dir.clone();
            let beatmapset_id = beatmapset.id;
            let pb = pb.clone();
            let shutdown = shutdown.clone();

            async move {
                if shutdown.load(Ordering::SeqCst) {
                    return (beatmapset_id, downloader::DownloadResult::Aborted);
                }

                let result = downloader::download_beatmap(
                    &client,
                    beatmapset_id,
                    &mirror_url,
                    &output_dir,
                    skip_existing,
                    cli.yes,
                ).await;

                let result = result.unwrap_or_else(|e| {
                    downloader::DownloadResult::FailedDynamic(
                        format!("{}", e).into_boxed_str()
                    )
                });

                pb.inc(1);
                (beatmapset_id, result)
            }
        })
        .buffer_unordered(concurrent)
        .collect::<Vec<_>>()
        .await;

    pb.finish_and_clear();

    let mut downloaded_count: u16 = 0;
    let mut skipped_count: u16 = 0;
    let mut failed_count: u16 = 0;
    let mut failed_downloads: Vec<(u32, Box<str>)> = Vec::new();
    let mut aborted = false;

    for (beatmapset_id, result) in results {
        match result {
            downloader::DownloadResult::Success(filename) => {
                downloaded_count += 1;
                println!("\x1b[32m✓\x1b[0m Downloaded: {}", filename);
            }
            downloader::DownloadResult::Skipped(filename) => {
                skipped_count += 1;
                println!("\x1b[33m⚠\x1b[0m Skipped (existing): {}", filename);
            }
            downloader::DownloadResult::Failed(reason) => {
                failed_count += 1;
                failed_downloads.push((beatmapset_id, reason.into()));
                println!("\x1b[31m✗\x1b[0m Error downloading {}: {}", beatmapset_id, reason);
            }
            downloader::DownloadResult::FailedDynamic(reason) => {
                failed_count += 1;
                failed_downloads.push((beatmapset_id, reason.clone()));
                println!("\x1b[31m✗\x1b[0m Error downloading {}: {}", beatmapset_id, reason);
            }
            downloader::DownloadResult::Aborted => {
                aborted = true;
                println!("\x1b[33m⚠  Download process aborted by user\x1b[0m");
                break;
            }
        }
    }

    if !aborted {
        println!("\nCreating collection.db...");
        let db_collection_name = format!("{}-{}", collection.name, collection.id);
        match collection::create_collection_db(&collection, &db_collection_name, &output_dir) {
            Ok(()) => {
                println!("\x1b[32m✓\x1b[0m collection.db created successfully");
            }
            Err(e) => {
                println!("\x1b[33m⚠\x1b[0m Warning: Failed to create collection.db: {}", e);
            }
        }
    }

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
