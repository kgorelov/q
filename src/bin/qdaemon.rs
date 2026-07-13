use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use q::{
    get_spool_dir, load_config,
    JobInfo, JobInfoShort, JobSpec, JobStatus, Request, Response,
};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--worker" {
        let job_id: usize = args[2].parse().expect("Invalid job ID");
        run_worker(job_id);
        return;
    }

    run_daemon().await;
}

fn is_worker_pid_running(pid: u32) -> bool {
    let comm_path = format!("/proc/{}/comm", pid);
    if let Ok(comm) = fs::read_to_string(comm_path) {
        comm.trim() == "qdaemon"
    } else {
        false
    }
}

fn scan_jobs(spool_dir: &Path) -> Vec<JobInfo> {
    let mut jobs = Vec::new();
    if let Ok(entries) = fs::read_dir(spool_dir) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    let name = entry.file_name();
                    if let Some(name_str) = name.to_str() {
                        if let Ok(id) = name_str.parse::<usize>() {
                            if let Some(job_info) = read_job_info(spool_dir, id) {
                                jobs.push(job_info);
                            }
                        }
                    }
                }
            }
        }
    }
    jobs.sort_by_key(|j| j.id);
    jobs
}

fn read_job_info(spool_dir: &Path, id: usize) -> Option<JobInfo> {
    let job_dir = spool_dir.join(id.to_string());
    let spec_path = job_dir.join("spec.json");
    if !spec_path.exists() {
        return None;
    }

    let spec_str = fs::read_to_string(&spec_path).ok()?;
    let spec: JobSpec = serde_json::from_str(&spec_str).ok()?;

    let status_path = job_dir.join("status");
    let status = if status_path.exists() {
        fs::read_to_string(&status_path)
            .map(|s| JobStatus::from_str(&s))
            .unwrap_or(JobStatus::Failed { error: "Could not read status file".to_string() })
    } else {
        JobStatus::Queued
    };

    let pid_path = job_dir.join("pid");
    let pid = if pid_path.exists() {
        fs::read_to_string(&pid_path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    } else {
        None
    };

    let worker_pid_path = job_dir.join("worker_pid");
    let worker_pid = if worker_pid_path.exists() {
        fs::read_to_string(&worker_pid_path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    } else {
        None
    };

    let start_time_path = job_dir.join("start_time");
    let start_time = if start_time_path.exists() {
        fs::read_to_string(&start_time_path).ok().map(|s| s.trim().to_string())
    } else {
        None
    };

    let end_time_path = job_dir.join("end_time");
    let end_time = if end_time_path.exists() {
        fs::read_to_string(&end_time_path).ok().map(|s| s.trim().to_string())
    } else {
        None
    };

    Some(JobInfo {
        id,
        spec,
        status,
        pid,
        worker_pid,
        start_time,
        end_time,
    })
}

fn get_next_job_id(spool_dir: &Path) -> usize {
    let mut max_id = 0;
    if let Ok(entries) = fs::read_dir(spool_dir) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    if let Some(name_str) = entry.file_name().to_str() {
                        if let Ok(id) = name_str.parse::<usize>() {
                            if id > max_id {
                                max_id = id;
                            }
                        }
                    }
                }
            }
        }
    }
    max_id + 1
}

fn apply_retention_policy(spool_dir: &Path, max_completed: usize) {
    let mut completed_jobs = Vec::new();
    if let Ok(entries) = fs::read_dir(spool_dir) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    let name = entry.file_name();
                    if let Some(name_str) = name.to_str() {
                        if let Ok(id) = name_str.parse::<usize>() {
                            let status_path = entry.path().join("status");
                            if status_path.exists() {
                                if let Ok(status_str) = fs::read_to_string(&status_path) {
                                    let status = JobStatus::from_str(&status_str);
                                    match status {
                                        JobStatus::Completed { .. }
                                        | JobStatus::Failed { .. }
                                        | JobStatus::Cancelled => {
                                            completed_jobs.push(id);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if completed_jobs.len() > max_completed {
        completed_jobs.sort_unstable();
        let to_remove = completed_jobs.len() - max_completed;
        for id in completed_jobs.iter().take(to_remove) {
            let job_dir = spool_dir.join(id.to_string());
            let _ = fs::remove_dir_all(job_dir);
        }
    }
}

fn recover_and_monitor_jobs(spool_dir: &Path, tx: mpsc::Sender<()>) -> Vec<usize> {
    let mut active_orphans = Vec::new();
    let jobs = scan_jobs(spool_dir);
    for job in jobs {
        if job.status == JobStatus::Running {
            let mut is_running = false;
            if let Some(wpid) = job.worker_pid {
                if is_worker_pid_running(wpid) {
                    is_running = true;
                    active_orphans.push(job.id);
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        while is_worker_pid_running(wpid) {
                            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        }
                        let _ = tx_clone.send(()).await;
                    });
                }
            }

            if !is_running {
                let job_dir = spool_dir.join(job.id.to_string());
                let _ = fs::write(job_dir.join("status"), "failed: process died while daemon was offline");
                let end_time = chrono::Local::now().to_rfc3339();
                let _ = fs::write(job_dir.join("end_time"), &end_time);
                let _ = fs::remove_file(job_dir.join("pid"));
                let _ = fs::remove_file(job_dir.join("worker_pid"));
            }
        }
    }
    active_orphans
}

async fn run_queue_manager(
    spool_dir: PathBuf,
    tx: mpsc::Sender<()>,
    mut rx: mpsc::Receiver<()>,
) {
    let qdaemon_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("qdaemon"));

    loop {
        let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await;

        let config = load_config();
        let jobs = scan_jobs(&spool_dir);

        let mut running_count = 0;
        let mut queued_jobs = Vec::new();

        for job in &jobs {
            match job.status {
                JobStatus::Running => {
                    let mut is_running = false;
                    if let Some(wpid) = job.worker_pid {
                        if is_worker_pid_running(wpid) {
                            is_running = true;
                        }
                    }
                    if is_running {
                        running_count += 1;
                    } else {
                        let job_dir = spool_dir.join(job.id.to_string());
                        let _ = fs::write(job_dir.join("status"), "failed: process died while daemon was offline");
                        let end_time = chrono::Local::now().to_rfc3339();
                        let _ = fs::write(job_dir.join("end_time"), &end_time);
                        let _ = fs::remove_file(job_dir.join("pid"));
                        let _ = fs::remove_file(job_dir.join("worker_pid"));
                    }
                }
                JobStatus::Queued => {
                    queued_jobs.push(job.clone());
                }
                _ => {}
            }
        }

        if running_count < config.max_parallel_jobs {
            let limit = config.max_parallel_jobs - running_count;
            for job in queued_jobs.into_iter().take(limit) {
                let job_dir = spool_dir.join(job.id.to_string());

                match tokio::process::Command::new(&qdaemon_exe)
                    .arg("--worker")
                    .arg(job.id.to_string())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    Ok(mut child) => {
                        let tx_clone = tx.clone();
                        tokio::spawn(async move {
                            let _ = child.wait().await;
                            let _ = tx_clone.send(()).await;
                        });
                    }
                    Err(e) => {
                        let _ = fs::write(job_dir.join("status"), format!("failed: cannot spawn worker: {}", e));
                    }
                }
            }
        }

        apply_retention_policy(&spool_dir, config.max_completed_jobs);
    }
}

async fn run_daemon() {
    let q_dir = q::get_q_dir();
    let spool_dir = q::get_spool_dir();
    let socket_path = q::get_socket_path();
    let pid_path = q::get_daemon_pid_path();

    let _ = fs::create_dir_all(&q_dir);
    let _ = fs::create_dir_all(&spool_dir);

    if socket_path.exists() {
        if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
            eprintln!("qdaemon is already running.");
            std::process::exit(1);
        } else {
            let _ = fs::remove_file(&socket_path);
        }
    }

    let my_pid = std::process::id();
    if let Err(e) = fs::write(&pid_path, my_pid.to_string()) {
        eprintln!("Failed to write daemon pid file: {}", e);
        std::process::exit(1);
    }

    let (tx, rx) = mpsc::channel::<()>(100);

    let active_orphans = recover_and_monitor_jobs(&spool_dir, tx.clone());
    println!("Recovered {} running orphaned jobs", active_orphans.len());

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to socket: {}", e);
            let _ = fs::remove_file(&pid_path);
            std::process::exit(1);
        }
    };

    let spool_dir_clone = spool_dir.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        run_queue_manager(spool_dir_clone, tx_clone, rx).await;
    });

    let socket_path_cleanup = socket_path.clone();
    let pid_path_cleanup = pid_path.clone();
    tokio::spawn(async move {
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).unwrap();
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();

        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }

        println!("Shutting down daemon...");
        let _ = fs::remove_file(&socket_path_cleanup);
        let _ = fs::remove_file(&pid_path_cleanup);
        std::process::exit(0);
    });

    println!("qdaemon started and listening on {:?}", socket_path);
    let _ = tx.send(()).await;

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let spool_dir = spool_dir.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    handle_connection(stream, spool_dir, tx).await;
                });
            }
            Err(e) => {
                eprintln!("Error accepting connection: {}", e);
            }
        }
    }
}

async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    spool_dir: PathBuf,
    tx: mpsc::Sender<()>,
) {
    let (reader, mut writer) = stream.split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    while let Ok(n) = buf_reader.read_line(&mut line).await {
        if n == 0 {
            break;
        }

        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::Error { message: format!("Invalid request JSON: {}", e) };
                if let Ok(resp_str) = serde_json::to_string(&resp) {
                    let _ = writer.write_all(format!("{}\n", resp_str).as_bytes()).await;
                }
                line.clear();
                continue;
            }
        };

        let response = match req {
            Request::Queue { cmd, args, work_dir, env } => {
                let job_id = get_next_job_id(&spool_dir);
                let job_dir = spool_dir.join(job_id.to_string());
                if let Err(e) = fs::create_dir_all(&job_dir) {
                    Response::Error { message: format!("Failed to create job directory: {}", e) }
                } else {
                    let spec = JobSpec { cmd, args, work_dir, env };
                    let spec_path = job_dir.join("spec.json");
                    let status_path = job_dir.join("status");

                    if let Err(e) = fs::write(&spec_path, serde_json::to_string(&spec).unwrap()) {
                        Response::Error { message: format!("Failed to write spec.json: {}", e) }
                    } else if let Err(e) = fs::write(&status_path, "queued") {
                        Response::Error { message: format!("Failed to write status: {}", e) }
                    } else {
                        let cmd_str = format!("{} {}", spec.cmd, spec.args.join(" "));
                        let _ = fs::write(job_dir.join("cmd"), cmd_str);

                        let _ = tx.send(()).await;
                        Response::Queued { job_id }
                    }
                }
            }
            Request::List => {
                let jobs = scan_jobs(&spool_dir);
                let jobs_short: Vec<JobInfoShort> = jobs
                    .into_iter()
                    .map(|j| JobInfoShort {
                        id: j.id,
                        cmd: format!("{} {}", j.spec.cmd, j.spec.args.join(" ")),
                        status: j.status.to_string(),
                        pid: j.pid,
                        start_time: j.start_time,
                        end_time: j.end_time,
                    })
                    .collect();
                Response::List { jobs: jobs_short }
            }
            Request::Kill { job_id } => {
                let job_dir = spool_dir.join(job_id.to_string());
                if !job_dir.exists() {
                    Response::Error { message: format!("Job {} does not exist", job_id) }
                } else {
                    let status_path = job_dir.join("status");
                    let status_str = fs::read_to_string(&status_path).unwrap_or_default();
                    let status = JobStatus::from_str(&status_str);

                    match status {
                        JobStatus::Queued => {
                            let _ = fs::write(&status_path, "cancelled");
                            let _ = tx.send(()).await;
                            Response::Ok
                        }
                        JobStatus::Running => {
                            let pid_path = job_dir.join("pid");
                            let worker_pid_path = job_dir.join("worker_pid");

                            let pid = fs::read_to_string(&pid_path)
                                .ok()
                                .and_then(|s| s.trim().parse::<u32>().ok());
                            let wpid = fs::read_to_string(&worker_pid_path)
                                .ok()
                                .and_then(|s| s.trim().parse::<u32>().ok());

                            if let Some(w) = wpid {
                                unsafe {
                                    libc::kill(w as libc::pid_t, libc::SIGKILL);
                                }
                            }
                            if let Some(p) = pid {
                                unsafe {
                                    libc::kill(p as libc::pid_t, libc::SIGKILL);
                                }
                            }

                            let _ = fs::write(&status_path, "cancelled");
                            let end_time = chrono::Local::now().to_rfc3339();
                            let _ = fs::write(job_dir.join("end_time"), &end_time);
                            let _ = fs::remove_file(pid_path);
                            let _ = fs::remove_file(worker_pid_path);

                            let _ = tx.send(()).await;
                            Response::Ok
                        }
                        _ => Response::Error {
                            message: format!("Job {} is not active (status: {})", job_id, status),
                        },
                    }
                }
            }
        };

        if let Ok(resp_str) = serde_json::to_string(&response) {
            let _ = writer.write_all(format!("{}\n", resp_str).as_bytes()).await;
        }

        line.clear();
    }
}

fn run_worker(job_id: usize) {
    let spool_dir = get_spool_dir();
    let job_dir = spool_dir.join(job_id.to_string());
    let spec_path = job_dir.join("spec.json");
    let pid_path = job_dir.join("pid");
    let worker_pid_path = job_dir.join("worker_pid");
    let status_path = job_dir.join("status");
    let stdout_path = job_dir.join("stdout");
    let stderr_path = job_dir.join("stderr");

    let my_pid = std::process::id();
    if let Err(e) = fs::write(&worker_pid_path, my_pid.to_string()) {
        let _ = fs::write(&status_path, format!("failed: cannot write worker_pid: {}", e));
        return;
    }

    let spec_str = match fs::read_to_string(&spec_path) {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::write(&status_path, format!("failed: cannot read spec.json: {}", e));
            return;
        }
    };
    let spec: JobSpec = match serde_json::from_str(&spec_str) {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::write(&status_path, format!("failed: cannot parse spec.json: {}", e));
            return;
        }
    };

    let stdout_file = match fs::File::create(&stdout_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::write(&status_path, format!("failed: cannot create stdout file: {}", e));
            return;
        }
    };
    let stderr_file = match fs::File::create(&stderr_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::write(&status_path, format!("failed: cannot create stderr file: {}", e));
            return;
        }
    };

    let mut cmd = std::process::Command::new(&spec.cmd);
    cmd.args(&spec.args)
        .current_dir(&spec.work_dir)
        .stdout(stdout_file)
        .stderr(stderr_file);

    for (k, v) in spec.env {
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::write(&status_path, format!("failed: {}", e));
            let _ = fs::remove_file(&worker_pid_path);
            return;
        }
    };

    let cmd_pid = child.id();
    if let Err(e) = fs::write(&pid_path, cmd_pid.to_string()) {
        let _ = fs::write(&status_path, format!("failed: cannot write pid file: {}", e));
        let _ = child.kill();
        let _ = fs::remove_file(&worker_pid_path);
        return;
    }

    if let Err(_e) = fs::write(&status_path, "running") {
        let _ = child.kill();
        let _ = fs::remove_file(&worker_pid_path);
        let _ = fs::remove_file(&pid_path);
        return;
    }

    let start_time = chrono::Local::now().to_rfc3339();
    let _ = fs::write(job_dir.join("start_time"), &start_time);

    let exit_status = match child.wait() {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::write(&status_path, format!("failed: waiting on process failed: {}", e));
            let end_time = chrono::Local::now().to_rfc3339();
            let _ = fs::write(job_dir.join("end_time"), &end_time);
            let _ = fs::remove_file(&worker_pid_path);
            let _ = fs::remove_file(&pid_path);
            return;
        }
    };

    let end_time = chrono::Local::now().to_rfc3339();
    let _ = fs::write(job_dir.join("end_time"), &end_time);

    let status_str = if let Some(code) = exit_status.code() {
        format!("completed {}", code)
    } else {
        "failed: killed by signal".to_string()
    };

    let _ = fs::write(&status_path, status_str);
    let _ = fs::remove_file(&worker_pid_path);
    let _ = fs::remove_file(&pid_path);
}
