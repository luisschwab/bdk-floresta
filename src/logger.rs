// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Logger
//!
//! This module provides a lightweight, opinionated logger built on top of
//! [`tracing-appender`] and [`tracing-subscriber`].
//!
//! ## Behavior
//!
//! - **Log level**: controlled by the `RUST_LOG` environment variable, falling back to
//!   [`LoggerConfig::log_level`] if unset.
//! - **Target formatting**: at `INFO` and above, module paths are shortened to a human-friendly
//!   prefix (e.g. `floresta_wire::p2p_wire::node` → `wire`). At `DEBUG` and below, the full module
//!   path is shown.
//! - **Color**: ANSI colors are applied when writing to an interactive terminal, and stripped when
//!   writing to a file.
//! - **Output**: events can be emitted to `stdout`, a log file, or both.
//!
//! ## Example
//!
//! ```rust,ignore
//! let _guard = start_logger(
//!     &data_dir,
//!     None,
//!     true,  // log to stdout
//!     true,  // log to file
//!     Level::INFO,
//! )?;
//! ```
//!
//! The returned [`WorkerGuard`] must be kept alive for the duration of the
//! program; dropping it will flush and stop the background logging thread.
//!
//! [`tracing-appender`]: https://crates.io/crates/tracing-appender
//! [`tracing-subscriber`]: https://crates.io/crates/tracing-subscriber

use core::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::layer;
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::FormatEvent;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;

use crate::error::BuilderError;

/// The default filename used for the log file when none is specified.
pub const LOG_FILE: &str = "debug.log";

/// Configuration for the logger.
///
/// Controls the log level and output destinations. Can be passed to the
/// [`Builder`](crate::Builder) or used directly with [`start_logger`].
pub struct LoggerConfig {
    /// The minimum log level to emit. Can be overridden at runtime via `RUST_LOG`.
    pub log_level: Level,
    /// Whether to emit log events to `stdout`.
    pub log_to_stdout: bool,
    /// Whether to emit log events to a file.
    pub log_to_file: bool,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            log_level: Level::INFO,
            log_to_stdout: true,
            log_to_file: true,
        }
    }
}

/// A custom [`FormatEvent`] implementation for [`tracing-subscriber`]'s `fmt` layer.
///
/// Formats log events as:
/// ```text
/// [YYYY-MM-DD HH:MM:SS] LEVEL target: message
/// ```
///
/// ## Target shortening
///
/// At `INFO` level and above, well-known `floresta_*` crate prefixes are
/// replaced with short aliases for readability:
///
/// | Module prefix                 | Alias     |
/// |-------------------------------|-----------|
/// | `floresta_chain`              | `chain`   |
/// | `floresta_compact_filters`    | `filters` |
/// | `floresta_mempool`            | `mempool` |
/// | `floresta_wire`               | `wire`    |
///
/// At `DEBUG` level and below, the full module path is preserved.
///
/// ## ANSI colors
///
/// When writing to an interactive terminal, log levels are colorized.
/// Colors are suppressed when writing to a file.
///
/// [`tracing-subscriber`]: https://crates.io/crates/tracing-subscriber
pub struct ShortTargetFormatter {
    timer: ChronoLocal,
}

impl Default for ShortTargetFormatter {
    fn default() -> Self {
        Self {
            timer: ChronoLocal::new("[%Y-%m-%d %H:%M:%S]".to_string()),
        }
    }
}

impl ShortTargetFormatter {
    /// Maps a full module path to a short human-friendly alias.
    ///
    /// Returns the original target unchanged if no alias is defined for it.
    fn short_target(target: &str) -> &str {
        if target.starts_with("floresta_chain") {
            "floresta::chain"
        } else if target.starts_with("floresta_compact_filters") {
            "floresta::compact_filters"
        } else if target.starts_with("floresta_mempool") {
            "floresta::mempool"
        } else if target.starts_with("floresta_wire") {
            "floresta::wire"
        } else {
            target
        }
    }
}

impl<S, N> FormatEvent<S, N> for ShortTargetFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        // Timestamp (ANSI colored if the TTY supports it).
        if writer.has_ansi_escapes() {
            write!(writer, "\x1b[2m")?;
        }
        self.timer.format_time(&mut writer)?;
        if writer.has_ansi_escapes() {
            write!(writer, "\x1b[0m ")?;
        } else {
            write!(writer, " ")?;
        }

        // Level (ANSI colored if the TTY supports it).
        let meta = event.metadata();
        if writer.has_ansi_escapes() {
            let colored_level = match *meta.level() {
                Level::ERROR => "\x1b[0;31mERROR\x1b[0m", // bold red
                Level::WARN => "\x1b[0;33m WARN\x1b[0m",  // bold yellow
                Level::INFO => "\x1b[0;32m INFO\x1b[0m",  // dimmed green
                Level::DEBUG => "\x1b[0;34mDEBUG\x1b[0m", // dimmed blue
                Level::TRACE => "\x1b[0;35mTRACE\x1b[0m", // dimmed magenta
            };
            write!(writer, "{} ", colored_level)?;
        } else {
            write!(writer, "{:>5} ", meta.level())?;
        }

        // Target (ANSI colored if the TTY supports it).
        let meta = event.metadata();
        let target = if tracing::enabled!(Level::DEBUG) {
            meta.target()
        } else {
            Self::short_target(meta.target())
        };
        if writer.has_ansi_escapes() {
            write!(writer, "\x1b[2m{}\x1b[0m: ", target)?;
        } else {
            write!(writer, "{}: ", target)?;
        }

        // Log Message and Fields.
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Start a logger that consumes tracing events and outputs them to `stdout` and/or a file.
///
/// The log level is read from the `RUST_LOG` environment variable if set,
/// otherwise `log_level` is used as the default.
///
/// # Returns
///
/// Returns a [`WorkerGuard`] that must be kept alive for the duration of the
/// program. Dropping it will flush and shut down the background logging thread.
///
/// # Errors
///
/// Returns a [`BuilderError`] if the log file cannot be created or opened.
pub fn start_logger(
    data_directory: &PathBuf,
    log_file: Option<String>,
    log_to_stdout: bool,
    log_to_file: bool,
    log_level: Level,
) -> Result<Option<WorkerGuard>, BuilderError> {
    let make_filter = || {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level.to_string()))
    };

    // Formatter for events destined to `stdout`.
    let ansi_tty = io::IsTerminal::is_terminal(&io::stdout());
    let fmt_layer_stdout = log_to_stdout.then(|| {
        layer()
            .with_writer(io::stdout)
            .with_ansi(ansi_tty)
            .event_format(ShortTargetFormatter::default())
            .with_filter(make_filter())
    });

    let log_file = log_file.unwrap_or(LOG_FILE.to_string());

    // Validate the log file path.
    if log_to_file {
        let file_path = format!("{}/{}", data_directory.to_string_lossy(), log_file);
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;
    }

    // Formatter for events destined to file.
    let mut guard = None;
    let fmt_layer_logfile = log_to_file.then(|| {
        let file_appender = tracing_appender::rolling::never(data_directory, log_file);
        let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);
        guard = Some(file_guard);
        layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .event_format(ShortTargetFormatter::default())
            .with_filter(make_filter())
    });

    tracing_subscriber::registry()
        .with(fmt_layer_stdout)
        .with(fmt_layer_logfile)
        .init();

    Ok(guard)
}
