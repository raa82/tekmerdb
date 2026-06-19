// TekmerDB Logger
// Appends to log/server.log. Rotation is handled externally by tekmerdb-cron.
// Also mirrors to stdout for live development visibility.
//
// Usage:
//   log_info!("message {}", value);
//   log_warn!("message {}", value);
//   log_error!("message {}", value);

use std::fs;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::fs::OpenOptions;
use chrono::Utc;

pub struct Logger {
    file: Mutex<std::fs::File>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

impl Logger {
    fn new(log_dir: &str, log_file: &str) -> std::io::Result<Self> {
        fs::create_dir_all(log_dir)?;
        let path = format!("{}/{}", log_dir, log_file);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Logger {
            file: Mutex::new(file),
        })
    }

    pub fn write(&self, level: &str, message: &str) {
        let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let line = format!("{} [{}] {}\n", ts, level, message);

        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }

        print!("{}", line);
    }
}

// initialise the global logger — call once from main()
pub fn init(log_dir: &str, log_file: &str) {
    let logger = Logger::new(log_dir, log_file)
        .expect("failed to initialise logger");
    LOGGER.set(logger).ok();
}

// internal write — called by macros
pub fn _write(level: &str, message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.write(level, message);
    } else {
        // fallback before logger is initialised
        println!("[{}] {}", level, message);
    }
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::engine::logger::_write("INFO", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::engine::logger::_write("WARN", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::engine::logger::_write("ERROR", &format!($($arg)*))
    };
}