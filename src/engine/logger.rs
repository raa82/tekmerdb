// TekmerDB Logger
// Writes to a daily-rotated log file in log/ directory.
// Also mirrors to stdout for live development visibility.
//
// Usage:
//   log_info!("message {}", value);
//   log_warn!("message {}", value);
//   log_error!("message {}", value);

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use chrono::Utc;

pub struct Logger {
    log_dir: String,
    log_file: String,
    current_date: Mutex<String>,
    file: Mutex<std::fs::File>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

impl Logger {
    fn new(log_dir: &str, log_file: &str) -> std::io::Result<Self> {
        fs::create_dir_all(log_dir)?;
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let path = format!("{}/{}", log_dir, log_file);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Logger {
            log_dir: log_dir.to_string(),
            log_file: log_file.to_string(),
            current_date: Mutex::new(today),
            file: Mutex::new(file),
        })
    }

    fn rotate_if_needed(&self) {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let mut current = self.current_date.lock().unwrap();
        if *current != today {
            // rotate: rename current log to dated archive
            let current_path = format!("{}/{}", self.log_dir, self.log_file);
            let archive_path = format!("{}/{}.{}", self.log_dir, self.log_file, *current);
            let _ = fs::rename(&current_path, &archive_path);

            // open new log file
            if let Ok(new_file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&current_path)
            {
                let mut file = self.file.lock().unwrap();
                *file = new_file;
            }
            *current = today;
        }
    }

    pub fn write(&self, level: &str, message: &str) {
        self.rotate_if_needed();
        let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let line = format!("{} [{}] {}\n", ts, level, message);

        // write to file
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }

        // mirror to stdout for live development
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