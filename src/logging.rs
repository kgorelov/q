use chrono::{DateTime, Local, NaiveDateTime};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::{get_daemon_log_path, Config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

pub fn parse_size_str(input: &str) -> Option<u64> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }

    let units: &[(&[&str], f64)] = &[
        (&["terabytes", "terabyte", "tib", "tb", "t"], 1024.0 * 1024.0 * 1024.0 * 1024.0),
        (&["gigabytes", "gigabyte", "gib", "gb", "g"], 1024.0 * 1024.0 * 1024.0),
        (&["megabytes", "megabyte", "mib", "mb", "m"], 1024.0 * 1024.0),
        (&["kilobytes", "kilobyte", "kib", "kb", "k"], 1024.0),
        (&["bytes", "byte", "b"], 1.0),
    ];

    for (aliases, multiplier) in units {
        for alias in *aliases {
            if s.ends_with(alias) {
                let num_part = s[..s.len() - alias.len()].trim();
                if let Ok(val) = num_part.parse::<f64>() {
                    if val >= 0.0 {
                        let bytes = (val * multiplier).round();
                        if bytes <= u64::MAX as f64 {
                            return Some(bytes as u64);
                        }
                    }
                }
                return None;
            }
        }
    }

    // Try parsing as plain number (bytes)
    if let Ok(val) = s.parse::<u64>() {
        return Some(val);
    }

    None
}

pub fn parse_age_str(input: &str) -> Option<u64> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }

    let units: &[(&[&str], f64)] = &[
        (&["years", "year", "yrs", "yr", "y"], 365.0 * 86400.0),
        (&["months", "month", "mon"], 30.0 * 86400.0),
        (&["weeks", "week", "wks", "wk", "w"], 7.0 * 86400.0),
        (&["days", "day", "d"], 86400.0),
        (&["hours", "hour", "hrs", "hr", "h"], 3600.0),
        (&["minutes", "minute", "mins", "min", "m"], 60.0),
        (&["seconds", "second", "secs", "sec", "s"], 1.0),
    ];

    for (aliases, multiplier) in units {
        for alias in *aliases {
            if s.ends_with(alias) {
                let num_part = s[..s.len() - alias.len()].trim();
                if let Ok(val) = num_part.parse::<f64>() {
                    if val >= 0.0 {
                        let secs = (val * multiplier).round();
                        if secs <= u64::MAX as f64 {
                            return Some(secs as u64);
                        }
                    }
                }
                return None;
            }
        }
    }

    // Try parsing as plain number (seconds)
    if let Ok(val) = s.parse::<u64>() {
        return Some(val);
    }

    None
}

pub fn rotated_log_path(log_path: &Path, index: usize) -> PathBuf {
    let file_name = log_path.file_name().unwrap_or_default().to_string_lossy();
    let rotated_name = format!("{}.{}", file_name, index);
    log_path.with_file_name(rotated_name)
}

pub fn get_file_first_log_time(log_path: &Path) -> Option<DateTime<Local>> {
    let file = File::open(log_path).ok()?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).ok()? > 0 {
        let trimmed = first_line.trim_start_matches('[').trim();
        if trimmed.len() >= 19 {
            let dt_str = &trimmed[0..19];
            if let Ok(naive) = NaiveDateTime::parse_from_str(dt_str, "%Y-%m-%d %H:%M:%S") {
                if let Some(local_dt) = naive.and_local_timezone(Local).single() {
                    return Some(local_dt);
                }
            }
        }
    }
    None
}

pub fn should_rotate(
    log_path: &Path,
    max_age_secs: Option<u64>,
    max_size_bytes: Option<u64>,
) -> bool {
    if !log_path.exists() {
        return false;
    }

    let metadata = match fs::metadata(log_path) {
        Ok(m) => m,
        Err(_) => return false,
    };

    if metadata.len() == 0 {
        return false;
    }

    if let Some(max_size) = max_size_bytes {
        if max_size > 0 && metadata.len() >= max_size {
            return true;
        }
    }

    if let Some(max_age) = max_age_secs {
        if max_age > 0 {
            let now = Local::now();
            if let Some(first_time) = get_file_first_log_time(log_path) {
                let elapsed = now.signed_duration_since(first_time).num_seconds();
                if elapsed >= max_age as i64 {
                    return true;
                }
            } else if let Ok(created) = metadata.created() {
                if let Ok(elapsed) = std::time::SystemTime::now().duration_since(created) {
                    if elapsed.as_secs() >= max_age {
                        return true;
                    }
                }
            } else if let Ok(modified) = metadata.modified() {
                if let Ok(elapsed) = std::time::SystemTime::now().duration_since(modified) {
                    if elapsed.as_secs() >= max_age {
                        return true;
                    }
                }
            }
        }
    }

    false
}

pub fn rotate_logs(log_path: &Path, max_log_files: usize) -> std::io::Result<()> {
    // 1. Remove old rotated log files beyond max_log_files
    let parent = log_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = log_path.file_name().unwrap_or_default().to_string_lossy();
    let prefix = format!("{}.", file_name);
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(suffix) = name_str.strip_prefix(&prefix) {
                if let Ok(idx) = suffix.parse::<usize>() {
                    if idx > max_log_files || max_log_files == 0 {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }

    if max_log_files == 0 {
        if log_path.exists() {
            let _ = fs::remove_file(log_path);
        }
        return Ok(());
    }

    // 2. Remove oldest allowed index if it exists
    let oldest = rotated_log_path(log_path, max_log_files);
    if oldest.exists() {
        let _ = fs::remove_file(&oldest);
    }

    // 3. Shift existing logs: log.N-1 -> log.N down to log.1 -> log.2
    for i in (1..max_log_files).rev() {
        let src = rotated_log_path(log_path, i);
        let dst = rotated_log_path(log_path, i + 1);
        if src.exists() {
            if dst.exists() {
                let _ = fs::remove_file(&dst);
            }
            let _ = fs::rename(&src, &dst);
        }
    }

    // 4. Move active log -> log.1
    if log_path.exists() {
        let dst = rotated_log_path(log_path, 1);
        if dst.exists() {
            let _ = fs::remove_file(&dst);
        }
        let _ = fs::rename(log_path, &dst);
    }

    Ok(())
}

pub struct DaemonLogger {
    log_path: PathBuf,
    file: Mutex<Option<File>>,
}

impl DaemonLogger {
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            log_path,
            file: Mutex::new(None),
        }
    }

    pub fn log(&self, level: LogLevel, message: &str, config: &Config) {
        let now = Local::now();
        let timestamp = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let line = format!("{} [{}] {}\n", timestamp, level.as_str(), message);

        let mut file_guard = match self.file.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        // Check if rotation needed
        let max_age = config.parsed_max_log_file_age_secs();
        let max_size = config.parsed_max_log_file_size_bytes();

        if should_rotate(&self.log_path, max_age, max_size) {
            // Close current file
            *file_guard = None;
            let _ = rotate_logs(&self.log_path, config.max_log_files);
        }

        if file_guard.is_none() {
            if let Some(parent) = self.log_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(f) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
            {
                *file_guard = Some(f);
            }
        }

        if let Some(f) = file_guard.as_mut() {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }

    pub fn check_and_rotate(&self, config: &Config) {
        let mut file_guard = match self.file.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        let max_age = config.parsed_max_log_file_age_secs();
        let max_size = config.parsed_max_log_file_size_bytes();

        if should_rotate(&self.log_path, max_age, max_size) {
            *file_guard = None;
            let _ = rotate_logs(&self.log_path, config.max_log_files);
        }
    }
}

static GLOBAL_LOGGER: OnceLock<DaemonLogger> = OnceLock::new();

pub fn get_logger() -> &'static DaemonLogger {
    GLOBAL_LOGGER.get_or_init(|| DaemonLogger::new(get_daemon_log_path()))
}

pub fn log(level: LogLevel, message: &str) {
    let config = crate::load_config();
    get_logger().log(level, message, &config);
}

pub fn check_and_rotate(config: &Config) {
    get_logger().check_and_rotate(config);
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::LogLevel::Info, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::LogLevel::Warn, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::LogLevel::Error, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::LogLevel::Debug, &format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_str() {
        assert_eq!(parse_size_str("200mb"), Some(200 * 1024 * 1024));
        assert_eq!(parse_size_str("200MB"), Some(200 * 1024 * 1024));
        assert_eq!(parse_size_str("200 mb"), Some(200 * 1024 * 1024));
        assert_eq!(parse_size_str("200Mb"), Some(200 * 1024 * 1024));
        assert_eq!(parse_size_str("200MiB"), Some(200 * 1024 * 1024));
        assert_eq!(parse_size_str("200 megabytes"), Some(200 * 1024 * 1024));
        assert_eq!(parse_size_str("500k"), Some(500 * 1024));
        assert_eq!(parse_size_str("500kb"), Some(500 * 1024));
        assert_eq!(parse_size_str("500 kib"), Some(500 * 1024));
        assert_eq!(parse_size_str("500 kilobytes"), Some(500 * 1024));
        assert_eq!(parse_size_str("1g"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size_str("1gb"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size_str("1 gigabyte"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size_str("100b"), Some(100));
        assert_eq!(parse_size_str("100 bytes"), Some(100));
        assert_eq!(parse_size_str("1048576"), Some(1048576));
        assert_eq!(parse_size_str("1.5mb"), Some(1572864));
        assert_eq!(parse_size_str("0"), Some(0));
        assert_eq!(parse_size_str(""), None);
        assert_eq!(parse_size_str("invalid"), None);
        assert_eq!(parse_size_str("-10mb"), None);
    }

    #[test]
    fn test_parse_age_str() {
        assert_eq!(parse_age_str("1w"), Some(604800));
        assert_eq!(parse_age_str("1 week"), Some(604800));
        assert_eq!(parse_age_str("2w"), Some(1209600));
        assert_eq!(parse_age_str("2 weeks"), Some(1209600));
        assert_eq!(parse_age_str("24h"), Some(86400));
        assert_eq!(parse_age_str("24 hours"), Some(86400));
        assert_eq!(parse_age_str("1 hr"), Some(3600));
        assert_eq!(parse_age_str("1d"), Some(86400));
        assert_eq!(parse_age_str("7 days"), Some(604800));
        assert_eq!(parse_age_str("1y"), Some(31536000));
        assert_eq!(parse_age_str("1 year"), Some(31536000));
        assert_eq!(parse_age_str("2 years"), Some(63072000));
        assert_eq!(parse_age_str("30m"), Some(1800));
        assert_eq!(parse_age_str("30 mins"), Some(1800));
        assert_eq!(parse_age_str("10s"), Some(10));
        assert_eq!(parse_age_str("10 seconds"), Some(10));
        assert_eq!(parse_age_str("1.5d"), Some(129600));
        assert_eq!(parse_age_str("604800"), Some(604800));
        assert_eq!(parse_age_str(""), None);
        assert_eq!(parse_age_str("invalid"), None);
        assert_eq!(parse_age_str("-5d"), None);
    }

    #[test]
    fn test_log_rotation_file_shifting() {
        let temp_dir = std::env::temp_dir().join(format!("q_test_rot_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("qdaemon.log");

        // Create log file
        fs::write(&log_file, "log 0").unwrap();

        // 1st rotation (max_log_files = 3)
        rotate_logs(&log_file, 3).unwrap();
        assert!(!log_file.exists());
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 1)).unwrap(), "log 0");

        // Create log file again
        fs::write(&log_file, "log 1").unwrap();

        // 2nd rotation
        rotate_logs(&log_file, 3).unwrap();
        assert!(!log_file.exists());
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 1)).unwrap(), "log 1");
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 2)).unwrap(), "log 0");

        // Create log file again
        fs::write(&log_file, "log 2").unwrap();

        // 3rd rotation
        rotate_logs(&log_file, 3).unwrap();
        assert!(!log_file.exists());
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 1)).unwrap(), "log 2");
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 2)).unwrap(), "log 1");
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 3)).unwrap(), "log 0");

        // Create log file again
        fs::write(&log_file, "log 3").unwrap();

        // 4th rotation (max_log_files = 3) -> "log 0" should be dropped
        rotate_logs(&log_file, 3).unwrap();
        assert!(!log_file.exists());
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 1)).unwrap(), "log 3");
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 2)).unwrap(), "log 2");
        assert_eq!(fs::read_to_string(rotated_log_path(&log_file, 3)).unwrap(), "log 1");
        assert!(!rotated_log_path(&log_file, 4).exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_log_rotation_max_zero() {
        let temp_dir = std::env::temp_dir().join(format!("q_test_rot_zero_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("qdaemon.log");

        fs::write(&log_file, "test log").unwrap();
        fs::write(rotated_log_path(&log_file, 1), "old 1").unwrap();

        rotate_logs(&log_file, 0).unwrap();
        assert!(!log_file.exists());
        assert!(!rotated_log_path(&log_file, 1).exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_should_rotate_size() {
        let temp_dir = std::env::temp_dir().join(format!("q_test_rot_size_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("qdaemon.log");

        fs::write(&log_file, "hello world 12345").unwrap(); // 17 bytes
        assert!(should_rotate(&log_file, None, Some(10)));
        assert!(!should_rotate(&log_file, None, Some(50)));
        assert!(!should_rotate(&log_file, None, None));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_should_rotate_age() {
        let temp_dir = std::env::temp_dir().join(format!("q_test_rot_age_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("qdaemon.log");

        // Write a log entry with a timestamp from 2 hours ago
        let old_time = Local::now() - chrono::Duration::hours(2);
        let log_line = format!("{} [INFO] Old entry\n", old_time.format("%Y-%m-%d %H:%M:%S"));
        fs::write(&log_file, log_line).unwrap();

        // max_age = 1 hour (3600s) -> should rotate
        assert!(should_rotate(&log_file, Some(3600), None));
        // max_age = 5 hours (18000s) -> should not rotate
        assert!(!should_rotate(&log_file, Some(18000), None));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_daemon_logger_auto_rotate() {
        let temp_dir = std::env::temp_dir().join(format!("q_test_logger_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("qdaemon.log");

        let logger = DaemonLogger::new(log_file.clone());
        let mut config = Config::default();
        config.max_log_files = 2;
        config.max_log_file_size = Some("50b".to_string()); // rotate after 50 bytes

        logger.log(LogLevel::Info, "First log message with enough length to test", &config);
        assert!(log_file.exists());

        // Log another message, exceeding 50 bytes
        logger.log(LogLevel::Info, "Second log message triggering rotation", &config);

        // After rotation, log_file.1 should exist and log_file should contain the new message
        let rot1 = rotated_log_path(&log_file, 1);
        assert!(rot1.exists());
        assert!(log_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
