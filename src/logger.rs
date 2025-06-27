use std::fmt::Arguments;

use fern::colors::{Color, ColoredLevelConfig};
use fern::{Dispatch, FormatCallback};
use log::{LevelFilter, Record};

pub fn setup_logger(debug: bool) -> Result<(), fern::InitError> {
    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Green)
        .debug(Color::Blue);

    let formatter = |use_colors: bool| {
        move |out: FormatCallback, message: &Arguments, record: &Record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                if use_colors {
                    colors.color(record.level()).to_string()
                } else {
                    record.level().to_string()
                },
                record.target(),
                message
            ))
        }
    };

    let allowed_crates = ["bdk_floresta", "example", "floresta_chain", "floresta_wire"];

    let filter = move |metadata: &log::Metadata| {
        allowed_crates
            .iter()
            .any(|prefix| metadata.target().starts_with(prefix))
    };

    let dispatch = Dispatch::new()
        .format(formatter(true))
        .level(LevelFilter::Trace)
        .filter(Box::new(filter))
        .level_for(
            "bdk_floresta",
            if debug {
                LevelFilter::Debug
            } else {
                LevelFilter::Info
            },
        )
        .level_for(
            "example",
            if debug {
                LevelFilter::Debug
            } else {
                LevelFilter::Info
            },
        )
        .level_for("floresta_chain", LevelFilter::Info)
        .level_for("floresta_wire", LevelFilter::Warn)
        .chain(std::io::stdout());

    dispatch.apply()?;

    Ok(())
}
