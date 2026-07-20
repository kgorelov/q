use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use q::{
    get_spool_dir, connect_daemon, ConnectionStream, JobInfoShort, Request, Response,
};

#[cfg(windows)]
const DAEMON_BIN: &str = "qdaemon.exe";
#[cfg(not(windows))]
const DAEMON_BIN: &str = "qdaemon";

#[cfg(unix)]
fn spawn_daemon(daemon_exe: &Path) -> std::io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    unsafe {
        std::process::Command::new(daemon_exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .pre_exec(|| {
                libc::setsid();
                Ok(())
            })
            .spawn()
    }
}

#[cfg(windows)]
fn spawn_daemon(daemon_exe: &Path) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(daemon_exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        handle_list().await;
        return;
    }

    let mut notify_override: Option<bool> = None;
    let mut idx = 1;

    while idx < args.len() {
        let arg = &args[idx];
        if arg == "-h" || arg == "--help" {
            print_help();
            return;
        } else if arg == "-l" || arg == "--list" {
            handle_list().await;
            return;
        } else if arg == "-k" || arg == "--kill" {
            if idx + 1 >= args.len() {
                eprintln!("Error: job ID is required.");
                eprintln!("Usage: q --kill <jobid>");
                std::process::exit(1);
            }
            let job_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid job ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            handle_kill(job_id).await;
            return;
        } else if arg == "-L" || arg == "--logs" {
            if idx + 1 >= args.len() {
                eprintln!("Error: job ID is required.");
                eprintln!("Usage: q --logs <jobid>");
                std::process::exit(1);
            }
            let job_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid job ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            handle_logs(job_id);
            return;
        } else if arg == "-n" || arg == "--notify" {
            notify_override = Some(true);
            idx += 1;
        } else if arg == "--no-notify" {
            notify_override = Some(false);
            idx += 1;
        } else {
            break;
        }
    }

    if idx >= args.len() {
        handle_list().await;
        return;
    }

    let cmd = args[idx].clone();
    let cmd_args = args[idx + 1..].to_vec();
    handle_queue(cmd, cmd_args, notify_override).await;
}

fn print_help() {
    println!("q - command line tool to queue and execute commands");
    println!();
    println!("Usage:");
    println!("  q [options]");
    println!("  q [notification-options] <command> [args...]");
    println!();
    println!("Options:");
    println!("  -l, --list        List all queued, running, and completed jobs");
    println!("  -k, --kill <id>   Kill a running job or cancel a queued job");
    println!("  -L, --logs <id>   Print stdout and stderr of a job");
    println!("  -n, --notify      Force desktop notification on job completion");
    println!("  --no-notify       Disable desktop notification for job completion");
    println!("  -h, --help        Show this help message");
}

async fn connect_or_start_daemon() -> ConnectionStream {
    if let Ok(stream) = connect_daemon().await {
        return stream;
    }

    // Daemon is not running, let's start it
    let current_exe = std::env::current_exe().ok();
    let daemon_exe = current_exe
        .as_ref()
        .map(|p| p.parent().unwrap().join(DAEMON_BIN))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(DAEMON_BIN));

    println!("Starting qdaemon...");
    let spawn_result = spawn_daemon(&daemon_exe);
    match spawn_result {
        Ok(_) => {
            // Poll socket to wait for daemon to start listening
            for _ in 0..60 {
                if let Ok(stream) = connect_daemon().await {
                    return stream;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
            eprintln!("Error: daemon started but connection did not become available.");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error: failed to start daemon (tried executable: {:?}): {}", daemon_exe, e);
            std::process::exit(1);
        }
    }
}

async fn handle_queue(cmd: String, args: Vec<String>, notify: Option<bool>) {
    let mut stream = connect_or_start_daemon().await;

    let work_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let env: Vec<(String, String)> = std::env::vars().collect();

    let req = Request::Queue { cmd, args, work_dir, env, notify };
    let req_str = format!("{}\n", serde_json::to_string(&req).unwrap());

    if let Err(e) = stream.write_all(req_str.as_bytes()).await {
        eprintln!("Error sending request to daemon: {}", e);
        std::process::exit(1);
    }

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    if let Err(e) = reader.read_line(&mut response_line).await {
        eprintln!("Error reading response from daemon: {}", e);
        std::process::exit(1);
    }

    let resp: Response = match serde_json::from_str(&response_line) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error parsing response from daemon: {}", e);
            std::process::exit(1);
        }
    };

    match resp {
        Response::Queued { job_id } => {
            println!("Job {} queued successfully.", job_id);
        }
        Response::Error { message } => {
            eprintln!("Error: {}", message);
            std::process::exit(1);
        }
        _ => {
            eprintln!("Unexpected response from daemon.");
            std::process::exit(1);
        }
    }
}

async fn handle_list() {
    let mut stream = connect_or_start_daemon().await;

    let req = Request::List;
    let req_str = format!("{}\n", serde_json::to_string(&req).unwrap());

    if let Err(e) = stream.write_all(req_str.as_bytes()).await {
        eprintln!("Error sending request to daemon: {}", e);
        std::process::exit(1);
    }

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    if let Err(e) = reader.read_line(&mut response_line).await {
        eprintln!("Error reading response from daemon: {}", e);
        std::process::exit(1);
    }

    let resp: Response = match serde_json::from_str(&response_line) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error parsing response from daemon: {}", e);
            std::process::exit(1);
        }
    };

    match resp {
        Response::List { mut jobs } => {
            if jobs.is_empty() {
                println!("No jobs in queue.");
                return;
            }

            // Sort jobs: Running (priority 0), Queued (priority 1), others (priority 2), then by ID ascending
            jobs.sort_by(|a, b| {
                let prio_a = get_status_priority(&a.status);
                let prio_b = get_status_priority(&b.status);
                if prio_a != prio_b {
                    prio_a.cmp(&prio_b)
                } else {
                    a.id.cmp(&b.id)
                }
            });

            print_jobs_table(&jobs);
        }
        Response::Error { message } => {
            eprintln!("Error: {}", message);
            std::process::exit(1);
        }
        _ => {
            eprintln!("Unexpected response from daemon.");
            std::process::exit(1);
        }
    }
}

fn get_status_priority(status: &str) -> usize {
    if status == "running" {
        0
    } else if status == "queued" {
        1
    } else {
        2
    }
}

fn format_duration(seconds: i64) -> String {
    if seconds < 0 {
        return "0s".to_string();
    }
    let secs = seconds % 60;
    let mins = (seconds / 60) % 60;
    let hours = (seconds / 3600) % 24;
    let days = seconds / 86400;

    if days > 0 {
        format!("{}d {}h {}m {}s", days, hours, mins, secs)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

fn print_jobs_table(jobs: &[JobInfoShort]) {
    let mut max_id_len = 6;
    let mut max_status_len = 8;
    let mut max_pid_len = 5;
    let mut max_start_len = 10; // "START TIME" length
    let mut max_time_len = 4; // "TIME" length

    let mut formatted_jobs = Vec::new();
    for job in jobs {
        let pid_str = job.pid.map(|p| p.to_string()).unwrap_or_default();

        let start_str = if let Some(ref start_time) = job.start_time {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(start_time) {
                dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()
            } else {
                "--".to_string()
            }
        } else {
            "--".to_string()
        };

        let duration_str = if let Some(ref start_time) = job.start_time {
            if let Ok(start_dt) = chrono::DateTime::parse_from_rfc3339(start_time) {
                let start_utc = start_dt.with_timezone(&chrono::Utc);
                let end_utc = if let Some(ref end_time) = job.end_time {
                    chrono::DateTime::parse_from_rfc3339(end_time)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now())
                } else {
                    chrono::Utc::now()
                };
                let diff = end_utc.signed_duration_since(start_utc);
                format_duration(diff.num_seconds())
            } else {
                "--".to_string()
            }
        } else {
            "--".to_string()
        };

        max_id_len = max_id_len.max(job.id.to_string().len());
        max_status_len = max_status_len.max(job.status.len());
        max_pid_len = max_pid_len.max(pid_str.len());
        max_start_len = max_start_len.max(start_str.len());
        max_time_len = max_time_len.max(duration_str.len());

        formatted_jobs.push((
            job.id,
            job.status.clone(),
            pid_str,
            start_str,
            duration_str,
            job.cmd.clone(),
        ));
    }

    println!(
        "{:<id_width$}  {:<status_width$}  {:<pid_width$}  {:<start_width$}  {:<time_width$}  {}",
        "JOB ID", "STATUS", "PID", "START TIME", "TIME", "COMMAND",
        id_width = max_id_len,
        status_width = max_status_len,
        pid_width = max_pid_len,
        start_width = max_start_len,
        time_width = max_time_len
    );
    println!(
        "{:-<id_width$}--{:-<status_width$}--{:-<pid_width$}--{:-<start_width$}--{:-<time_width$}--{:-<20}",
        "", "", "", "", "", "",
        id_width = max_id_len,
        status_width = max_status_len,
        pid_width = max_pid_len,
        start_width = max_start_len,
        time_width = max_time_len
    );

    for (id, status, pid_str, start_str, duration_str, cmd) in formatted_jobs {
        println!(
            "{:<id_width$}  {:<status_width$}  {:<pid_width$}  {:<start_width$}  {:<time_width$}  {}",
            id, status, pid_str, start_str, duration_str, cmd,
            id_width = max_id_len,
            status_width = max_status_len,
            pid_width = max_pid_len,
            start_width = max_start_len,
            time_width = max_time_len
        );
    }
}

async fn handle_kill(job_id: usize) {
    let mut stream = connect_or_start_daemon().await;

    let req = Request::Kill { job_id };
    let req_str = format!("{}\n", serde_json::to_string(&req).unwrap());

    if let Err(e) = stream.write_all(req_str.as_bytes()).await {
        eprintln!("Error sending request to daemon: {}", e);
        std::process::exit(1);
    }

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    if let Err(e) = reader.read_line(&mut response_line).await {
        eprintln!("Error reading response from daemon: {}", e);
        std::process::exit(1);
    }

    let resp: Response = match serde_json::from_str(&response_line) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error parsing response from daemon: {}", e);
            std::process::exit(1);
        }
    };

    match resp {
        Response::Ok => {
            println!("Job {} killed or cancelled successfully.", job_id);
        }
        Response::Error { message } => {
            eprintln!("Error: {}", message);
            std::process::exit(1);
        }
        _ => {
            eprintln!("Unexpected response from daemon.");
            std::process::exit(1);
        }
    }
}

fn handle_logs(job_id: usize) {
    let spool_dir = get_spool_dir();
    let job_dir = spool_dir.join(job_id.to_string());
    if !job_dir.exists() {
        eprintln!("Job {} not found.", job_id);
        std::process::exit(1);
    }

    let stdout_path = job_dir.join("stdout");
    if stdout_path.exists() {
        if let Ok(mut file) = fs::File::open(&stdout_path) {
            let _ = io::copy(&mut file, &mut io::stdout());
        }
    }

    let stderr_path = job_dir.join("stderr");
    if stderr_path.exists() {
        if let Ok(mut file) = fs::File::open(&stderr_path) {
            let _ = io::copy(&mut file, &mut io::stderr());
        }
    }
}
