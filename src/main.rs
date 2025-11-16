mod app;
mod config;
mod core;
mod download;
mod mirrors;
mod tui;
mod utils;
mod worker;

#[cfg(windows)]
mod windows_init;
use app::run_app;
use config::ConfigService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    windows_init::enable_ansi_support();

    let config_service = ConfigService::new();
    let config = config_service.load_or_default();
    let _logging_guard = utils::init_logging(&config.logging)?;
    run_app(config).await
}
