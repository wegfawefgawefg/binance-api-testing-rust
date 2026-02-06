use chrono::Local;
#[allow(unused_imports)]
use env_logger::Builder;
use fern::colors::{Color, ColoredLevelConfig};
use fern::Dispatch;
use log::LevelFilter;
#[allow(unused_imports)]
use std::io::Write;

pub fn init_logging() {
    // Define color configuration for different log levels
    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Green)
        .debug(Color::Cyan)
        .trace(Color::BrightBlack);

    // Configure fern
    Dispatch::new()
        .format(move |out, message, record| {
            // Timestamp
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");

            // Retrieve source file, line, and module path
            let file = record.file().unwrap_or("unknown_file");
            let line = record.line().unwrap_or(0);
            // let module_path = record.module_path().unwrap_or("unknown_module");

            // Apply color to the log level
            let level = colors.color(record.level());

            // Write the formatted log message
            out.finish(format_args!(
                "{} {} [{}:{}] - {}",
                timestamp,
                level,
                file,
                line,
                message // "{} {} [{}:{}] [{}] - {}",
                        // timestamp, level, file, line, module_path, message
            ))
        })
        .level(LevelFilter::Info) // Set global log level
        .chain(std::io::stdout()) // Log to stdout
        .chain(fern::log_file("output.log").unwrap()) // Also log to a file
        .apply()
        .unwrap();
}
