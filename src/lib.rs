use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

