// MCP client logger — appends to log/mcp-client.log.
// Rotation is handled externally by tekmerdb-cron.
// Mirrors to stderr (MCP uses stdout for JSON-RPC — never print there).

use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use chrono::Utc;

struct McpLogger {
    file: Mutex<std::fs::File>,
}

static MCP_LOGGER: OnceLock<McpLogger> = OnceLock::new();

impl McpLogger {
    fn new(log_dir: &str, log_file: &str) -> std::io::Result<Self> {
        fs::create_dir_all(log_dir)?;
        let path = format!("{}/{}", log_dir, log_file);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(McpLogger {
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