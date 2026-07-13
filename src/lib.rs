use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub max_parallel_jobs: usize,
    pub max_completed_jobs: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_parallel_jobs: 2,
            max_completed_jobs: 50,
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
