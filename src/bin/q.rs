use std::fs;
use std::io;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use q::{
    get_socket_path, get_spool_dir, JobInfoShort, Request, Response,
};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        handle_list().await;
        return;
    }

    let first_arg = &args[1];

    if first_arg == "-h" || first_arg == "--help" {
        print_help();
        return;
    }

    if first_arg == "-l" || first_arg == "--list" {
        handle_list().await;
        return;
    }

    if first_arg == "-k" || first_arg == "--kill" {
        if args.len() < 3 {
            eprintln!("Error: job ID is required.");
            eprintln!("Usage: q --kill <jobid>");
            std::process::exit(1);
        }
        let job_id: usize = match args[2].parse() {
            Ok(id) => id,
            Err(_) => {
                eprintln!("Error: invalid job ID '{}'", args[2]);
                std::process::exit(1);
            }
        };
        handle_kill(job_id).await;
        return;
    }

    if first_arg == "--logs" {
        if args.len() < 3 {
            eprintln!("Error: job ID is required.");
            eprintln!("Usage: q --logs <jobid>");
            std::process::exit(1);
        }
        let job_id: usize = match args[2].parse() {
            Ok(id) => id,
            Err(_) => {
                eprintln!("Error: invalid job ID '{}'", args[2]);
                std::process::exit(1);
            }
        };
        handle_logs(job_id);
        return;
    }

    // Otherwise, queue a command
    let cmd = first_arg.clone();
    let cmd_args = args[2..].to_vec();
    handle_queue(cmd, cmd_args).await;
}

fn print_help() {
    println!("q - command line tool to queue and execute commands");
    println!();
    println!("Usage:");
    println!("  q [options]");
    println!("  q <command> [args...]");
    println!();
    println!("Options:");
    println!("  -l, --list        List all queued, running, and completed jobs");
    println!("  -k, --kill <id>   Kill a running job or cancel a queued job");
    println!("  --logs <id>       Print stdout and stderr of a job");
    println!("  -h, --help        Show this help message");
}

async fn connect_or_start_daemon() -> UnixStream {
    let socket_path = get_socket_path();

    if socket_path.exists() {
        if let Ok(stream) = UnixStream::connect(&socket_path).await {
            return stream;
        }
    }

    // Daemon is not running, let's start it
    let current_exe = std::env::current_exe().ok();
    let daemon_exe = current_exe
        .as_ref()
        .map(|p| p.parent().unwrap().join("qdaemon"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("qdaemon"));

    println!("Starting qdaemon...");
    use std::os::unix::process::CommandExt;
    let spawn_result = unsafe {
        std::process::Command::new(&daemon_exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .pre_exec(|| {
                libc::setsid();
                Ok(())
            })
            .spawn()
    };
    match spawn_result {
        Ok(_) => {
            // Poll socket to wait for daemon to start listening
            for _ in 0..60 {
                if let Ok(stream) = UnixStream::connect(&socket_path).await {
                    return stream;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
            eprintln!("Error: daemon started but socket did not become available.");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error: failed to start daemon (tried executable: {:?}): {}", daemon_exe, e);
            std::process::exit(1);
        }
    }
}

async fn handle_queue(cmd: String, args: Vec<String>) {
    let mut stream = connect_or_start_daemon().await;

    let work_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let env: Vec<(String, String)> = std::env::vars().collect();

    let req = Request::Queue { cmd, args, work_dir, env };
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

fn print_jobs_table(jobs: &[JobInfoShort]) {
    let mut max_id_len = 6;
    let mut max_status_len = 8;
    let mut max_pid_len = 5;

    for job in jobs {
        max_id_len = max_id_len.max(job.id.to_string().len());
        max_status_len = max_status_len.max(job.status.len());
        max_pid_len = max_pid_len.max(job.pid.map(|p| p.to_string().len()).unwrap_or(0));
    }

    println!(
        "{:<id_width$}  {:<status_width$}  {:<pid_width$}  {}",
        "JOB ID", "STATUS", "PID", "COMMAND",
        id_width = max_id_len,
        status_width = max_status_len,
        pid_width = max_pid_len
    );
    println!(
        "{:-<id_width$}--{:-<status_width$}--{:-<pid_width$}--{:-<20}",
        "", "", "", "",
        id_width = max_id_len,
        status_width = max_status_len,
        pid_width = max_pid_len
    );

    for job in jobs {
        let pid_str = job.pid.map(|p| p.to_string()).unwrap_or_default();
        println!(
            "{:<id_width$}  {:<status_width$}  {:<pid_width$}  {}",
            job.id, job.status, pid_str, job.cmd,
            id_width = max_id_len,
            status_width = max_status_len,
            pid_width = max_pid_len
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
