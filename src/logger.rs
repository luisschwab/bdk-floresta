// SPDX-License-Identifier: MIT

//! # Logger
//!
//! This module implements a basic logger using the [`tracing-subscriber`] crate.
//!
//! The [`tracing`] crate is used by [`bdk_floresta`], [`floresta-chain`],
//! [`floresta-compact-filters`], [`floresta-mempool`] and [`floresta-wire`]
//! to emit logging events. As such, users of [`bdk_floresta`] can also
//! implement their own logger to consume these events.
//!
//! [`bdk_floresta`]: https://github.com/luisschwab/bdk-floresta
//! [`floresta-chain`]: https://github.com/getfloresta/Floresta/tree/master/crates/floresta-wire
//! [`floresta-compact-filters`]: https://github.com/getfloresta/Floresta/tree/master/crates/floresta-compact-filters
//! [`floresta-mempool`]: https://github.com/getfloresta/Floresta/tree/master/crates/floresta-mempool
//! [`floresta-wire`]: https://github.com/getfloresta/Floresta/tree/master/crates/floresta-wire
//! [`tracing`]: https://crates.io/tracing
//! [`tracing-subscriber`]: https://crates.io/crates/tracing-subscriber

use std::io;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::layer::Layered;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::Registry;

use crate::error::BuilderError;

/// Build a logger that consumes tracing events from this crate and all underlying crates.
pub fn build_logger(
    data_dir: &str,
    log_to_file: bool,
    log_to_stdout: bool,
    debug: bool,
) -> Result<Option<WorkerGuard>, BuilderError> {
    // Set the log level to INFO. If `debug` is set, to DEBUG.
    let log_level = if debug { Level::DEBUG } else { Level::INFO };

    // Try to build an [`EnvFilter`] from the `RUST_LOG`
    // environment variable, or fallback to `log_level`.
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level.to_string()));

    // Apply custom log level filters depending on the source crate.
    let targets_log_filter: Targets = Targets::new()
        .with_target("bdk_floresta", log_level)
        .with_target("floresta_chain", Level::INFO)
        .with_target("floresta_wire", Level::INFO)
        .with_default(Level::INFO);

    // TODO(@luisschwab): add `tokio-console` dev dep + feature flag
    // For the registry, also enable very verbose runtime traces so
    // tokio-console works, but keep human outputs quiet via per-layer filters
    // below.
    //#[cfg(feature = "tokio-console")]
    //let base_filter = EnvFilter::new(format!("{},tokio=trace,runtime=trace",
    // log_filter));

    //#[cfg(not(feature = "tokio-console"))]
    let base_filter = targets_log_filter.clone();

    // Validate the log file path.
    if log_to_file {
        let file_path = format!("{data_dir}/debug.log");
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;
    }

    // Formatter for the event's timestamp.
    let log_time_formatter = ChronoLocal::new("%Y-%m-%d %H:%M:%S".to_string());

    // Formatter for events destined to standard output.
    let fmt_layer_stdout = log_to_stdout.then(|| {
        fmt::layer()
            .with_writer(io::stdout)
            .with_ansi(io::IsTerminal::is_terminal(&io::stdout()))
            .with_timer(log_time_formatter.clone())
            .with_target(true)
            .with_level(true)
            .with_filter(targets_log_filter.clone())
    });

    // Formatter for events destined to file.
    let mut guard = None;
    let fmt_layer_logfile = log_to_file.then(|| {
        let file_appender = tracing_appender::rolling::never(data_dir, "debug.log");
        let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);
        guard = Some(file_guard);
        fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .with_timer(log_time_formatter)
            .with_target(true)
            .with_level(true)
            .with_filter(targets_log_filter.clone())
    });

    // Build the registry with it's base filter, then attach layers to it.
    let registry: Layered<Targets, Registry> = tracing_subscriber::registry().with(base_filter);

    // Spawn the `console_subscriber` in the background
    // and apply it's [`Layer`] to the [`Registry`], if
    // the `tokio-console` feature is enabled.
    //#[cfg(feature = "tokio-console")]
    //let registry = registry.with(console_subscriber::spawn());

    // Apply the `stdout` and logfile's [`Layer`]s to the [`Registry`].
    registry
        .with(env_filter)
        .with(fmt_layer_stdout)
        .with(fmt_layer_logfile)
        .init();

    Ok(guard)
}
