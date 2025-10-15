// SPDX-License-Identifier: MIT

use std::io;
use std::io::IsTerminal;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    fmt, fmt::time::ChronoLocal, layer::SubscriberExt, util::SubscriberInitExt,
    EnvFilter, Layer,
};

use crate::error::BuilderError;

pub(crate) fn setup_logger(
    data_dir: &str,
    log_to_file: bool,
    log_to_stdout: bool,
    debug: bool,
) -> Result<Option<WorkerGuard>, BuilderError> {
    // Get the log level from `debug`.
    let log_level = if debug { "debug" } else { "info" };

    // Try to build an `EnvFilter` from the `RUST_LOG` environment variable, or
    // fallback to `log_level`.
    let log_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    // TODO(@luisschwab): add `tokio-console` dev dep + feature flag
    // For the registry, also enable very verbose runtime traces so
    // tokio-console works, but keep human outputs quiet via per-layer filters
    // below.
    //#[cfg(feature = "tokio-console")]
    //let base_filter = EnvFilter::new(format!("{},tokio=trace,runtime=trace",
    // log_filter));

    //#[cfg(not(feature = "tokio-console"))]
    let base_filter = log_filter.clone();

    // Validate the log file path.
    if log_to_file {
        let file_path = format!("{data_dir}/debug.log");
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;
    }

    // Timer for log events.
    let log_timer = ChronoLocal::new("%Y-%m-%d %H:%M:%S".to_string());

    // Formatting [`Layer`] for logs destined to `stdout`.
    let fmt_layer_stdout = log_to_stdout.then(|| {
        fmt::layer()
            .with_writer(io::stdout)
            .with_ansi(IsTerminal::is_terminal(&io::stdout()))
            .with_timer(log_timer.clone())
            .with_target(true)
            .with_level(true)
            .with_filter(log_filter.clone())
    });

    // Formatting [`Layer`] for logs destined to the log file.
    let mut guard = None;
    let fmt_layer_logfile = log_to_file.then(|| {
        let file_appender =
            tracing_appender::rolling::never(data_dir, "debug.log");
        let (non_blocking, file_guard) =
            tracing_appender::non_blocking(file_appender);
        guard = Some(file_guard);
        fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .with_timer(log_timer)
            .with_target(true)
            .with_level(true)
            .with_filter(log_filter.clone())
    });

    // Build the registry with its
    // (possibly more permissive) base filter, then attach layers to it.
    let registry = tracing_subscriber::registry().with(base_filter);

    // Spawn the `console_subscriber` in the background
    // and apply it's [`Layer`] to the [`Registry`], if
    // the `tokio-console` feature is enabled.
    //#[cfg(feature = "tokio-console")]
    //let registry = registry.with(console_subscriber::spawn());

    // Apply the `stdout` and logfile's [`Layer`]s to the [`Registry`].
    registry
        .with(fmt_layer_stdout)
        .with(fmt_layer_logfile)
        .init();

    Ok(guard)
}
