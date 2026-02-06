use chrono::Local;
use fern::colors::{Color, ColoredLevelConfig};
use fern::Dispatch;
use log::LevelFilter;

pub fn init_logging() {
    // Define color configuration for different log levels
    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Green)
        .debug(Color::Cyan)
        .trace(Color::BrightBlack);

    // ========================
    // 1. Configure Terminal Logging with Colors
    // ========================
    let stdout_dispatch = Dispatch::new()
        // Set the log level for terminal output
        .level(LevelFilter::Info)
        // Apply colored formatting
        .format(move |out, message, record| {
            // Timestamp
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");

            // Retrieve source file and line number
            let file = record.file().unwrap_or("unknown_file");
            let line = record.line().unwrap_or(0);

            // Apply color to the log level
            let level = colors.color(record.level());

            // Write the formatted log message
            out.finish(format_args!(
                "{} {} [{}:{}] - {}",
                timestamp, level, file, line, message
            ))
        })
        // Chain to standard output (terminal)
        .chain(std::io::stdout());

    // ========================
    // 2. Configure File Logging without Colors
    // ========================
    let _file_dispatch = Dispatch::new()
        // Set the log level for file output
        .level(LevelFilter::Info)
        // Apply plain formatting without colors
        .format(|out, message, record| {
            // Timestamp
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");

            // Retrieve source file and line number
            let file = record.file().unwrap_or("unknown_file");
            let line = record.line().unwrap_or(0);

            // Get the log level as plain text
            let level = record.level();

            // Write the formatted log message without color
            out.finish(format_args!(
                "{} {} [{}:{}] - {}",
                timestamp, level, file, line, message
            ))
        })
        // Chain to the log file
        .chain(fern::log_file("output.log").unwrap());

    // ========================
    // 3. Merge Both Dispatches
    // ========================
    Dispatch::new()
        // Merge the terminal and file dispatches
        .chain(stdout_dispatch)
        // .chain(file_dispatch)
        // Apply the combined configuration
        .apply()
        .unwrap();
}
