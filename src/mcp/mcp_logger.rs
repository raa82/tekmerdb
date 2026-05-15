// MCP client logger — mirrors logger.rs pattern for the MCP binary
// Writes to log/mcp-client.log with daily rotation

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use chrono::Utc;

struct McpLogger {
    log_dir: String,
    log_file: String,
    current_date: Mutex<String>,
    file: Mutex<std::fs::File>,
}

static MCP_LOGGER: OnceLock<McpLogger> = OnceLock::new();

impl McpLogger {
    fn new(log_dir: &str, log_file: &str) -> std::io::Result<Self> {
        fs::create_dir_all(log_dir)?;
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let path = format!("{}/{}", log_dir, log_file);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(McpLogger {
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
            let current_path = format!("{}/{}", self.log_dir, self.log_file);
            let archive_path = format!("{}/{}.{}", self.log_dir, self.log_file, *current);
            let _ = fs::rename(&current_path, &archive_path);
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
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        // mirror to stderr (MCP uses stdout for JSON-RPC — never print there)
        eprint!("{}", line);
    }
}

pub fn init() {
    let logger = McpLogger::new("log", "mcp-client.log")
        .expect("failed to initialise mcp logger");
    MCP_LOGGER.set(logger).ok();
}

pub fn _write(level: &str, message: &str) {
    if let Some(logger) = MCP_LOGGER.get() {
        logger.write(level, message);
    } else {
        eprintln!("[{}] {}", level, message);
    }
}

#[macro_export]
macro_rules! mcp_log_info {
    ($($arg:tt)*) => {
        $crate::mcp_logger::_write("INFO", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! mcp_log_warn {
    ($($arg:tt)*) => {
        $crate::mcp_logger::_write("WARN", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! mcp_log_error {
    ($($arg:tt)*) => {
        $crate::mcp_logger::_write("ERROR", &format!($($arg)*))
    };
}