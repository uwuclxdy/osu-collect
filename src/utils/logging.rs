use super::{AppError, Result};
use crate::config::{LogFormat, LogLevel, LoggingConfig};
use std::path::PathBuf;
use tracing::level_filters::LevelFilter;
use tracing_appender::{non_blocking, rolling};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub struct LoggingGuard {
    _file_guard: tracing_appender::non_blocking::WorkerGuard,
}

/// Initialize the global tracing subscriber based on configuration.
pub fn init_logging(config: &LoggingConfig) -> Result<Option<LoggingGuard>> {
    if !config.enabled {
        return Ok(None);
    }

    let guard = match config.format {
        LogFormat::Pretty => init_pretty_logging(config)?,
        LogFormat::Compact => init_compact_logging(config)?,
    };

    Ok(Some(guard))
}

fn init_compact_logging(config: &LoggingConfig) -> Result<LoggingGuard> {
    let env_filter = logging_env_filter(config);
    let (file_writer, file_guard) = build_file_writer(config)?;

    let console_layer = fmt::layer()
        .compact()
        .with_writer(std::io::stderr)
        .with_target(false);

    let file_layer = fmt::layer()
        .json()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true)
        .with_current_span(false)
        .with_span_events(fmt::format::FmtSpan::NONE);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .try_init()
        .map_err(|err| {
            AppError::other_dynamic(format!("failed to initialize logging subscriber: {}", err))
        })?;

    Ok(LoggingGuard {
        _file_guard: file_guard,
    })
}

fn init_pretty_logging(config: &LoggingConfig) -> Result<LoggingGuard> {
    let env_filter = logging_env_filter(config);
    let (file_writer, file_guard) = build_file_writer(config)?;

    let console_layer = fmt::layer()
        .pretty()
        .with_writer(std::io::stderr)
        .with_target(false);

    let file_layer = fmt::layer()
        .json()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true)
        .with_current_span(false)
        .with_span_events(fmt::format::FmtSpan::NONE);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .try_init()
        .map_err(|err| {
            AppError::other_dynamic(format!("failed to initialize logging subscriber: {}", err))
        })?;

    Ok(LoggingGuard {
        _file_guard: file_guard,
    })
}

fn logging_env_filter(config: &LoggingConfig) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(level_filter(config.level).into())
        .from_env_lossy()
}

fn build_file_writer(
    config: &LoggingConfig,
) -> Result<(
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
)> {
    let dir = resolve_log_dir(config)?;
    let file_appender = rolling::daily(dir, "osu-collect.log");
    Ok(non_blocking(file_appender))
}

fn resolve_log_dir(config: &LoggingConfig) -> Result<PathBuf> {
    let explicit = config.file_dir.as_deref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    });

    let path = if let Some(dir) = explicit {
        dir
    } else if let Some(base) = dirs::data_local_dir() {
        base.join("osu-collect").join("logs")
    } else {
        return Err(AppError::other(
            "logging.file_dir is not set and a data directory could not be determined",
        ));
    };

    if let Err(err) = std::fs::create_dir_all(&path) {
        return Err(AppError::filesystem_context_with_source(
            err,
            format!("failed to create log directory at {}", path.display()),
        ));
    }

    Ok(path)
}

fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    }
}
