pub mod timespec;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use timespec::TimeSpec;

fn default_true() -> bool {
    true
}

fn default_min_notify_duration() -> u64 {
    10
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub max_parallel_jobs: usize,
    pub max_completed_jobs: usize,
    #[serde(default = "default_true")]
    pub enable_notifications: bool,
    #[serde(default = "default_min_notify_duration")]
    pub min_notify_duration_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_parallel_jobs: 2,
            max_completed_jobs: 50,
            enable_notifications: true,
            min_notify_duration_secs: 10,
        }
    }
}

pub fn get_q_dir() -> PathBuf {
    dirs::home_dir()
        .map(|p| p.join(".q"))
        .unwrap_or_else(|| PathBuf::from(".q"))
}

pub fn get_spool_dir() -> PathBuf {
    get_q_dir().join("spool")
}

pub fn get_schedules_dir() -> PathBuf {
    get_q_dir().join("schedules")
}

pub fn get_socket_path() -> PathBuf {
    get_q_dir().join("q.sock")
}

pub fn get_daemon_pid_path() -> PathBuf {
    get_q_dir().join("qdaemon.pid")
}

pub fn get_port_path() -> PathBuf {
    get_q_dir().join("q.port")
}

#[cfg(unix)]
pub type ConnectionStream = tokio::net::UnixStream;

#[cfg(windows)]
pub type ConnectionStream = tokio::net::TcpStream;

#[cfg(unix)]
pub async fn connect_daemon() -> std::io::Result<ConnectionStream> {
    let socket_path = get_socket_path();
    tokio::net::UnixStream::connect(&socket_path).await
}

#[cfg(windows)]
pub async fn connect_daemon() -> std::io::Result<ConnectionStream> {
    let port_path = get_port_path();
    if !port_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Port file not found",
        ));
    }
    let content = fs::read_to_string(&port_path)?;
    let port: u16 = content
        .trim()
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::net::TcpStream::connect(("127.0.0.1", port)).await
}

#[cfg(unix)]
pub struct ConnectionListener {
    inner: tokio::net::UnixListener,
}

#[cfg(unix)]
impl ConnectionListener {
    pub async fn bind() -> std::io::Result<Self> {
        let socket_path = get_socket_path();
        let inner = tokio::net::UnixListener::bind(&socket_path)?;
        Ok(Self { inner })
    }

    pub async fn accept(&self) -> std::io::Result<ConnectionStream> {
        let (stream, _) = self.inner.accept().await?;
        Ok(stream)
    }
}

#[cfg(windows)]
pub struct ConnectionListener {
    inner: tokio::net::TcpListener,
}

#[cfg(windows)]
impl ConnectionListener {
    pub async fn bind() -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let port_path = get_port_path();
        fs::write(&port_path, port.to_string())?;
        Ok(Self { inner: listener })
    }

    pub async fn accept(&self) -> std::io::Result<ConnectionStream> {
        let (stream, _) = self.inner.accept().await?;
        Ok(stream)
    }
}

pub fn get_config_path() -> PathBuf {
    dirs::config_dir()
        .map(|p| p.join("q").join("q.conf"))
        .unwrap_or_else(|| get_q_dir().join("q.conf"))
}

pub fn load_config() -> Config {
    let path = get_config_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(config) = toml::from_str(&content) {
                return config;
            }
        }
    }
    Config::default()
}

pub fn send_notification(summary: &str, body: &str) {
    let result = notify_rust::Notification::new()
        .appname("q")
        .summary(summary)
        .body(body)
        .icon("utilities-terminal")
        .show();
    if let Err(e) = result {
        eprintln!("Failed to send desktop notification: {}", e);
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Completed { exit_code: i32 },
    Failed { error: String },
    Cancelled,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Queued => write!(f, "queued"),
            JobStatus::Running => write!(f, "running"),
            JobStatus::Completed { exit_code } => write!(f, "completed ({})", exit_code),
            JobStatus::Failed { error } => write!(f, "failed: {}", error),
            JobStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl JobStatus {
    pub fn from_str(s: &str) -> Self {
        let s = s.trim();
        if s == "queued" {
            JobStatus::Queued
        } else if s == "running" {
            JobStatus::Running
        } else if s == "cancelled" {
            JobStatus::Cancelled
        } else if s.starts_with("completed") {
            let mut code_str = s.strip_prefix("completed").unwrap().trim();
            if code_str.starts_with(':') {
                code_str = code_str.strip_prefix(':').unwrap().trim();
            }
            let code = code_str.parse::<i32>().unwrap_or(0);
            JobStatus::Completed { exit_code: code }
        } else if s.starts_with("failed") {
            let mut err = s.strip_prefix("failed").unwrap().trim();
            if err.starts_with(':') {
                err = err.strip_prefix(':').unwrap().trim();
            }
            JobStatus::Failed { error: err.to_string() }
        } else {
            JobStatus::Failed { error: format!("Unknown status format: {}", s) }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JobSpec {
    pub cmd: String,
    pub args: Vec<String>,
    pub work_dir: String,
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub notify: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JobInfo {
    pub id: usize,
    pub spec: JobSpec,
    pub status: JobStatus,
    pub pid: Option<u32>,
    pub worker_pid: Option<u32>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScheduleSpec {
    pub id: usize,
    pub timespec: String,
    pub parsed: TimeSpec,
    pub cmd: String,
    pub args: Vec<String>,
    pub work_dir: String,
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub notify: Option<bool>,
    pub created_at: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub enabled_at: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScheduleInfo {
    pub spec: ScheduleSpec,
    pub last_run: Option<String>,
    pub last_job_id: Option<usize>,
}

pub fn format_relative_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return "0s".to_string();
    }
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let mins = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        if hours > 0 {
            format!("{}d {}h", days, hours)
        } else {
            format!("{}d", days)
        }
    } else if hours > 0 {
        if mins > 0 {
            format!("{}h {}m", hours, mins)
        } else {
            format!("{}h", hours)
        }
    } else if mins > 0 {
        if secs > 0 {
            format!("{}m {}s", mins, secs)
        } else {
            format!("{}m", mins)
        }
    } else {
        format!("{}s", secs)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScheduleInfoShort {
    pub id: usize,
    pub timespec: String,
    pub cmd: String,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    #[serde(default)]
    pub is_running: bool,
    #[serde(default)]
    pub last_status: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
    Queue {
        cmd: String,
        args: Vec<String>,
        work_dir: String,
        env: Vec<(String, String)>,
        #[serde(default)]
        notify: Option<bool>,
    },
    List,
    Kill {
        job_id: usize,
    },
    Schedule {
        timespec: String,
        cmd: String,
        args: Vec<String>,
        work_dir: String,
        env: Vec<(String, String)>,
        #[serde(default)]
        notify: Option<bool>,
    },
    ScheduleList,
    ScheduleKill {
        schedule_id: usize,
    },
    ScheduleDisable {
        schedule_id: usize,
    },
    ScheduleEnable {
        schedule_id: usize,
    },
    ScheduleUpdate {
        schedule_id: usize,
        timespec: String,
    },
    ScheduleRun {
        schedule_id: usize,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JobInfoShort {
    pub id: usize,
    pub cmd: String,
    pub status: String,
    pub pid: Option<u32>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Response {
    Ok,
    Queued { job_id: usize },
    List { jobs: Vec<JobInfoShort> },
    Scheduled { schedule_id: usize },
    ScheduleList { schedules: Vec<ScheduleInfoShort> },
    Error { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    Always,
    Never,
    Auto,
}

impl ColorChoice {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "always" => Ok(ColorChoice::Always),
            "never" => Ok(ColorChoice::Never),
            "auto" => Ok(ColorChoice::Auto),
            _ => Err(format!(
                "invalid color argument '{}' (valid values: always, never, auto)",
                s
            )),
        }
    }

    pub fn should_color(self) -> bool {
        use std::io::IsTerminal;
        match self {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => std::io::stdout().is_terminal(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_choice_parse() {
        assert_eq!(ColorChoice::parse("always"), Ok(ColorChoice::Always));
        assert_eq!(ColorChoice::parse("ALWAYS"), Ok(ColorChoice::Always));
        assert_eq!(ColorChoice::parse("never"), Ok(ColorChoice::Never));
        assert_eq!(ColorChoice::parse("NEVER"), Ok(ColorChoice::Never));
        assert_eq!(ColorChoice::parse("auto"), Ok(ColorChoice::Auto));
        assert_eq!(ColorChoice::parse("AUTO"), Ok(ColorChoice::Auto));
        assert!(ColorChoice::parse("invalid").is_err());
    }

    #[test]
    fn test_config_defaults() {
        let toml_str = r#"
            max_parallel_jobs = 4
            max_completed_jobs = 100
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.max_parallel_jobs, 4);
        assert_eq!(config.max_completed_jobs, 100);
        assert!(config.enable_notifications);
        assert_eq!(config.min_notify_duration_secs, 10);
    }

    #[test]
    fn test_config_custom_notification_settings() {
        let toml_str = r#"
            max_parallel_jobs = 2
            max_completed_jobs = 50
            enable_notifications = false
            min_notify_duration_secs = 5
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.enable_notifications);
        assert_eq!(config.min_notify_duration_secs, 5);
    }

    #[test]
    fn test_job_spec_backward_compatibility() {
        let json_str = r#"{"cmd":"sleep","args":["5"],"work_dir":".","env":[]}"#;
        let spec: JobSpec = serde_json::from_str(json_str).unwrap();
        assert_eq!(spec.cmd, "sleep");
        assert_eq!(spec.notify, None);
    }

    #[test]
    fn test_job_spec_with_notify() {
        let json_str = r#"{"cmd":"sleep","args":["5"],"work_dir":".","env":[],"notify":true}"#;
        let spec: JobSpec = serde_json::from_str(json_str).unwrap();
        assert_eq!(spec.notify, Some(true));
    }

    #[test]
    fn test_schedule_request_serialization() {
        let req = Request::Schedule {
            timespec: "Wed 10 am".to_string(),
            cmd: "backup.sh".to_string(),
            args: vec!["--all".to_string()],
            work_dir: "/tmp".to_string(),
            env: vec![("FOO".to_string(), "BAR".to_string())],
            notify: Some(true),
        };
        let s = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&s).unwrap();
        match parsed {
            Request::Schedule { timespec, cmd, args, .. } => {
                assert_eq!(timespec, "Wed 10 am");
                assert_eq!(cmd, "backup.sh");
                assert_eq!(args, vec!["--all"]);
            }
            _ => panic!("Expected Schedule request"),
        }
    }

    #[test]
    fn test_format_relative_duration() {
        assert_eq!(format_relative_duration(0), "0s");
        assert_eq!(format_relative_duration(-5), "0s");
        assert_eq!(format_relative_duration(45), "45s");
        assert_eq!(format_relative_duration(60), "1m");
        assert_eq!(format_relative_duration(330), "5m 30s");
        assert_eq!(format_relative_duration(3600), "1h");
        assert_eq!(format_relative_duration(22 * 3600 + 11 * 60), "22h 11m");
        assert_eq!(format_relative_duration(22 * 3600), "22h");
        assert_eq!(format_relative_duration(86400 * 2), "2d");
        assert_eq!(format_relative_duration(86400 + 13 * 3600), "1d 13h");
        assert_eq!(format_relative_duration(86400 * 3 + 4 * 3600 + 20 * 60 + 10), "3d 4h");
    }

    #[test]
    fn test_schedule_info_short_backward_compatibility() {
        let json_str = r#"{"id":1,"timespec":"@hourly","cmd":"echo 1","last_run":null,"next_run":"2026-08-10T22:00:00Z"}"#;
        let info: ScheduleInfoShort = serde_json::from_str(json_str).unwrap();
        assert_eq!(info.id, 1);
        assert!(!info.is_running);
        assert_eq!(info.last_status, None);
    }
}

