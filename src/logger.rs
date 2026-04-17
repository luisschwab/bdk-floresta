// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Logger
//!
//! This module provides a lightweight, opinionated logger built on top of
//! [`tracing-appender`] and [`tracing-subscriber`].
//!
//! ## Behavior
//!
//! - **Log level**: controlled by the `RUST_LOG` environment variable, falling back to
//!   [`Logger::log_level`] if unset.
//! - **Target formatting**: at `INFO` and above, well-known `floresta_*` module paths are shortened
//!   to human-friendly aliases (e.g. `floresta_wire::p2p_wire::node` → `floresta::wire`). At
//!   `DEBUG` and below, the full module path is preserved.
//! - **Timestamp**: at `INFO` and above, timestamps are formatted as `YYYY-MM-DD HH:MM:SS`. At
//!   `DEBUG` and below, milliseconds are included: `YYYY-MM-DD HH:MM:SS.mmm`.
//! - **Color**: ANSI colors are applied when writing to an interactive terminal, and stripped when
//!   writing to a file.
//! - **Output**: events can be emitted to `stdout`, a log file, or both.
//!
//! ## Example
//!
//! ```rust,ignore
//! use bdk_floresta::logger::Logger;
//! use tracing::Level;
//!
//! // stdout only
//! Logger::default().init()?;
//!
//! // stdout + file
//! let _guard = Logger {
//!     log_level: Level::DEBUG,
//!     log_to_stdout: true,
//!     log_file: Some("./data/debug.log".into()),
//! }.init()?;
//! ```
//!
//! When logging to a file, the returned [`WorkerGuard`] must be kept alive
//! for the duration of the program — dropping it will flush and shut down
//! the background file-writing thread.
//!
//! [`tracing-appender`]: https://crates.io/crates/tracing-appender
//! [`tracing-subscriber`]: https://crates.io/crates/tracing-subscriber

use core::fmt;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
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

/// The file which logging events are written to by default.
pub const LOG_FILE: &str = "debug.log";

/// The string used to format the timestamp, via [`ChronoLocal`].
///
/// Format: `YYYY-MM-DD HH:mm:ss`
pub(crate) const CHRONO_FORMATTER: &str = "%Y-%m-%d %H:%M:%S";

/// The string used to format the timestamp when
/// the log level [`Level::DEBUG`] or higher, via [`ChronoLocal`].
///
/// Format: `YYYY-MM-DD HH:mm:ss.sss`
pub(crate) const CHRONO_FORMATTER_DEBUG: &str = "%Y-%m-%d %H:%M:%S%.3f";

/// ANSI escape code to begin dimmed text.
pub(crate) const ANSI_DIM: &str = "\x1b[2m";

/// ANSI escape code to reset all formatting.
pub(crate) const ANSI_RESET: &str = "\x1b[0m";

/// Colored `ERROR` in bright red.
pub(crate) const COLORED_ERROR: &str = "\x1b[0;31mERROR\x1b[0m";

/// Colored `WARN` in bright yellow
pub(crate) const COLORED_WARN: &str = "\x1b[0;33m WARN\x1b[0m";

/// Colored `INFO` in dim green.
pub(crate) const COLORED_INFO: &str = "\x1b[0;32m INFO\x1b[0m";

/// Colored `DEBUG` in dim blue.
pub(crate) const COLORED_DEBUG: &str = "\x1b[0;34mDEBUG\x1b[0m";

/// Colored `TRACE` in dim magenta.
pub(crate) const COLORED_TRACE: &str = "\x1b[0;35mTRACE\x1b[0m";

/// A custom [`FormatEvent`] implementation for [`tracing-subscriber`]'s `fmt` layer.
///
/// Formats log events as:
/// ```text
/// YYYY-MM-DD HH:MM:SS LEVEL target: message
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
            timer: ChronoLocal::new(CHRONO_FORMATTER.to_string()),
        }
    }
}

impl ShortTargetFormatter {
    /// Create a new [`ShortTargetFormatter`] for the given [`Level`].
    ///
    /// At [`Level::DEBUG`] and below, the timestamp will include milliseconds.
    pub fn new(level: Level) -> Self {
        let time_fmt = if level >= Level::DEBUG {
            CHRONO_FORMATTER_DEBUG
        } else {
            CHRONO_FORMATTER
        };

        Self {
            timer: ChronoLocal::new(time_fmt.to_string()),
        }
    }

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
        let event_metadata = event.metadata();

        // Check if the `writer` supports ANSI escape codes.
        let writer_supports_ansi_escaping = writer.has_ansi_escapes();

        // Timestamp (dimmed if the `tty` supports it).
        if writer_supports_ansi_escaping {
            write!(writer, "{}", ANSI_DIM)?;
        }
        self.timer.format_time(&mut writer)?;
        if writer_supports_ansi_escaping {
            write!(writer, "{} ", ANSI_RESET)?;
        } else {
            write!(writer, " ")?;
        }

        // Level (colored if the `tty` supports it).
        if writer_supports_ansi_escaping {
            let colored_level = match *event_metadata.level() {
                Level::ERROR => COLORED_ERROR,
                Level::WARN => COLORED_WARN,
                Level::INFO => COLORED_INFO,
                Level::DEBUG => COLORED_DEBUG,
                Level::TRACE => COLORED_TRACE,
            };
            write!(writer, "{} ", colored_level)?;
        } else {
            write!(writer, "{:>5} ", event_metadata.level())?;
        }

        // Target (dimmed if the TTY supports it).
        let target = if tracing::enabled!(Level::DEBUG) {
            event_metadata.target()
        } else {
            Self::short_target(event_metadata.target())
        };
        if writer_supports_ansi_escaping {
            write!(writer, "{}{}{}: ", ANSI_DIM, target, ANSI_RESET)?;
        } else {
            write!(writer, "{}: ", target)?;
        }

        // Log message and fields.
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Logger configuration for the [`Node`](crate::Node).
///
/// Controls the log level and output destinations. Can be passed to the
/// [`Builder`](crate::Builder) via [`Builder::with_logger`](crate::Builder::with_logger),
/// or used directly by calling [`Logger::init`].
///
/// # Example
///
/// ```rust,ignore
/// use bdk_floresta::logger::Logger;
/// use tracing::Level;
///
/// let _guard = Logger {
///     log_level: Level::DEBUG,
///     log_to_stdout: true,
///     log_file: Some("./data/debug.log".into()),
/// }.init()?;
/// ```
#[derive(Clone, Debug)]
pub struct Logger {
    /// The minimum log level to emit. Can be overridden at runtime via `RUST_LOG`.
    pub log_level: Level,
    /// Whether to emit log events to `stdout`.
    pub log_to_stdout: bool,
    /// The file where logging events should be written to.
    ///
    /// If `None`, log events will not be written to a file.
    pub log_file: Option<PathBuf>,
}

impl Default for Logger {
    /// Returns a [`Logger`] that emits [`Level::INFO`] events to `stdout` only.
    fn default() -> Self {
        Self {
            log_level: Level::INFO,
            log_to_stdout: true,
            log_file: None,
        }
    }
}

impl Logger {
    /// Initialize a global tracing subscriber with this configuration.
    ///
    /// The log level is read from the `RUST_LOG` environment variable,
    /// if set. Otherwise, [`Logger::log_level`] is used as the default.
    ///
    /// If a global subscriber is already registered, this will
    ///
    /// # Returns
    ///
    /// Returns an `Option<`[`WorkerGuard`]`>` which is `Some` only when
    /// [`Logger::log_file`] is set. The guard must be kept alive for the
    /// duration of the program — dropping it will flush and shut down the
    /// background file-writing thread.
    ///
    /// # Errors
    ///
    /// - Returns [`BuilderError::LoggerAlreadySetup`] if a global tracing subscriber has already
    ///   been registered.
    /// - Returns [`BuilderError::LoggerSetup`] if the log file cannot be created or opened.
    pub fn init(self) -> Result<Option<WorkerGuard>, BuilderError> {
        let make_filter = || {
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(self.log_level.to_string()))
        };

        // Formatter for events destined to `stdout`.
        let ansi_tty = io::IsTerminal::is_terminal(&io::stdout());
        let fmt_layer_stdout = self.log_to_stdout.then(|| {
            layer()
                .with_writer(io::stdout)
                .with_ansi(ansi_tty)
                .event_format(ShortTargetFormatter::new(self.log_level))
                .with_filter(make_filter())
        });

        // Formatter for events destined to file.
        let mut guard = None;
        let fmt_layer_logfile = self
            .log_file
            .map(|log_file| -> Result<_, BuilderError> {
                // Validate the log file path before handing it to the `tracing_appender`.
                if let Some(parent) = log_file.parent() {
                    fs::create_dir_all(parent).map_err(BuilderError::LoggerSetup)?;
                }
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_file)
                    .map_err(BuilderError::LoggerSetup)?;

                let file_appender = tracing_appender::rolling::never(
                    log_file.parent().unwrap_or(Path::new(".")),
                    log_file.file_name().unwrap_or(OsStr::new(LOG_FILE)),
                );
                let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);
                guard = Some(file_guard);
                Ok(layer()
                    .with_writer(non_blocking)
                    .with_ansi(false)
                    .event_format(ShortTargetFormatter::new(self.log_level))
                    .with_filter(make_filter()))
            })
            .transpose()?;

        tracing_subscriber::registry()
            .with(fmt_layer_stdout)
            .with(fmt_layer_logfile)
            .try_init()
            .map_err(|_| BuilderError::LoggerAlreadySetup)?;

        Ok(guard)
    }
}
