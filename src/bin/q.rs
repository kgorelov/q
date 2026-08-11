use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use q::{
    get_spool_dir, connect_daemon, load_config, ConnectionStream, JobInfoShort,
    ScheduleInfoShort, JobStatus, ColorChoice, Request, Response, format_relative_duration,
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

fn get_env_color_choice() -> ColorChoice {
    if let Ok(val) = std::env::var("Q_COLOR") {
        match ColorChoice::parse(&val) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error in Q_COLOR environment variable: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        ColorChoice::Auto
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let env_color = get_env_color_choice();

    // Check if invoked as `schedule` alias
    let is_schedule_alias = std::env::args()
        .next()
        .map(|p| {
            let path = PathBuf::from(p);
            path.file_stem()
                .map(|s| s == "schedule")
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if is_schedule_alias {
        run_schedule_cli(&args[1..], env_color).await;
        return;
    }

    let mut color_choice = env_color;
    let mut notify_override: Option<bool> = None;
    let mut is_list = false;
    let mut list_limit: Option<usize> = None;
    let mut idx = 1;

    while idx < args.len() {
        let arg = &args[idx];
        if arg == "-h" || arg == "--help" {
            print_help();
            return;
        } else if arg == "--color" {
            if idx + 1 < args.len() && (args[idx + 1] == "always" || args[idx + 1] == "never" || args[idx + 1] == "auto") {
                color_choice = ColorChoice::parse(&args[idx + 1]).unwrap();
                idx += 2;
            } else {
                color_choice = ColorChoice::Always;
                idx += 1;
            }
        } else if let Some(val) = arg.strip_prefix("--color=") {
            match ColorChoice::parse(val) {
                Ok(c) => color_choice = c,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            idx += 1;
        } else if arg == "-l" || arg == "--list" {
            is_list = true;
            if idx + 1 < args.len() {
                if let Ok(limit) = args[idx + 1].parse::<usize>() {
                    list_limit = Some(limit);
                    idx += 2;
                } else if !args[idx + 1].starts_with('-') {
                    eprintln!("Error: invalid number of jobs to print '{}'", args[idx + 1]);
                    std::process::exit(1);
                } else {
                    idx += 1;
                }
            } else {
                idx += 1;
            }
        } else if let Some(val) = arg.strip_prefix("--list=") {
            is_list = true;
            match val.parse::<usize>() {
                Ok(limit) => list_limit = Some(limit),
                Err(_) => {
                    eprintln!("Error: invalid number of jobs to print '{}'", val);
                    std::process::exit(1);
                }
            }
            idx += 1;
        } else if let Some(val) = arg.strip_prefix("-l=") {
            is_list = true;
            match val.parse::<usize>() {
                Ok(limit) => list_limit = Some(limit),
                Err(_) => {
                    eprintln!("Error: invalid number of jobs to print '{}'", val);
                    std::process::exit(1);
                }
            }
            idx += 1;
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
        } else if arg == "--run" {
            if idx + 1 >= args.len() {
                eprintln!("Error: schedule ID is required.");
                eprintln!("Usage: q --run <jobid>");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            handle_schedule_run(schedule_id).await;
            return;
        } else if arg == "--reschedule" {
            if idx + 2 >= args.len() {
                eprintln!("Error: schedule ID and timespec are required.");
                eprintln!("Usage: q --reschedule <jobid> <timespec>");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            let timespec = args[idx + 2].clone();
            handle_schedule_update(schedule_id, timespec).await;
            return;
        } else if arg == "-r" {
            if idx + 1 >= args.len() {
                eprintln!("Error: schedule ID is required.");
                eprintln!("Usage: q -r <jobid> [timespec]");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            if idx + 2 < args.len() {
                let timespec = args[idx + 2].clone();
                handle_schedule_update(schedule_id, timespec).await;
            } else {
                handle_schedule_run(schedule_id).await;
            }
            return;
        } else if arg == "-s" || arg == "--schedule" {
            // Schedule mode via `q --schedule` or `q -s`
            run_schedule_cli(&args[idx + 1..], color_choice).await;
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

    if is_list || idx >= args.len() {
        handle_list(color_choice.should_color(), list_limit).await;
        return;
    }

    let cmd = args[idx].clone();
    let cmd_args = args[idx + 1..].to_vec();
    handle_queue(cmd, cmd_args, notify_override).await;
}

async fn run_schedule_cli(args: &[String], default_color_choice: ColorChoice) {
    let mut color_choice = default_color_choice;
    let mut notify_override: Option<bool> = None;
    let mut is_list = false;
    let mut idx = 0;

    while idx < args.len() {
        let arg = &args[idx];
        if arg == "-h" || arg == "--help" {
            print_schedule_help();
            return;
        } else if arg == "--color" {
            if idx + 1 < args.len() && (args[idx + 1] == "always" || args[idx + 1] == "never" || args[idx + 1] == "auto") {
                color_choice = ColorChoice::parse(&args[idx + 1]).unwrap();
                idx += 2;
            } else {
                color_choice = ColorChoice::Always;
                idx += 1;
            }
        } else if let Some(val) = arg.strip_prefix("--color=") {
            match ColorChoice::parse(val) {
                Ok(c) => color_choice = c,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            idx += 1;
        } else if arg == "-l" || arg == "--list" {
            is_list = true;
            idx += 1;
        } else if arg == "-k" || arg == "--kill" {
            if idx + 1 >= args.len() {
                eprintln!("Error: schedule ID is required.");
                eprintln!("Usage: schedule --kill <id>");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            handle_schedule_kill(schedule_id).await;
            return;
        } else if arg == "-d" || arg == "--disable" {
            if idx + 1 >= args.len() {
                eprintln!("Error: schedule ID is required.");
                eprintln!("Usage: schedule --disable <id>");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            handle_schedule_disable(schedule_id).await;
            return;
        } else if arg == "-e" || arg == "--enable" {
            if idx + 1 >= args.len() {
                eprintln!("Error: schedule ID is required.");
                eprintln!("Usage: schedule --enable <id>");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            handle_schedule_enable(schedule_id).await;
            return;
        } else if arg == "--run" {
            if idx + 1 >= args.len() {
                eprintln!("Error: schedule ID is required.");
                eprintln!("Usage: schedule --run <id>");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            handle_schedule_run(schedule_id).await;
            return;
        } else if arg == "--reschedule" {
            if idx + 2 >= args.len() {
                eprintln!("Error: schedule ID and timespec are required.");
                eprintln!("Usage: schedule --reschedule <jobid> <timespec>");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            let timespec = args[idx + 2].clone();
            handle_schedule_update(schedule_id, timespec).await;
            return;
        } else if arg == "-r" {
            if idx + 1 >= args.len() {
                eprintln!("Error: schedule ID is required.");
                eprintln!("Usage: schedule -r <id> [timespec]");
                std::process::exit(1);
            }
            let schedule_id: usize = match args[idx + 1].parse() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("Error: invalid schedule ID '{}'", args[idx + 1]);
                    std::process::exit(1);
                }
            };
            if idx + 2 < args.len() {
                let timespec = args[idx + 2].clone();
                handle_schedule_update(schedule_id, timespec).await;
            } else {
                handle_schedule_run(schedule_id).await;
            }
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

    if is_list || idx >= args.len() {
        handle_schedule_list(color_choice.should_color()).await;
        return;
    }

    if idx + 1 >= args.len() {
        eprintln!("Error: command is required.");
        eprintln!("Usage: schedule <timespec> <command> [args...]");
        eprintln!("       q --schedule <timespec> <command> [args...]");
        std::process::exit(1);
    }

    let timespec = args[idx].clone();
    let cmd = args[idx + 1].clone();
    let cmd_args = args[idx + 2..].to_vec();
    handle_schedule_add(timespec, cmd, cmd_args, notify_override).await;
}

fn print_help() {
    println!("q - command line tool to queue, execute, and schedule commands");
    println!();
    println!("Usage:");
    println!("  q [options]");
    println!("  q [notification-options] <command> [args...]");
    println!("  q -s, --schedule [schedule-options]");
    println!("  q -s, --schedule <timespec> <command> [args...]");
    println!("  schedule [schedule-options]");
    println!("  schedule <timespec> <command> [args...]");
    println!();
    println!("Options:");
    println!("  -l, --list [N]              List queued, running, and completed jobs (up to N completed, default from config)");
    println!("  -k, --kill <id>             Kill a running job or cancel a queued job");
    println!("  -L, --logs <id>             Print stdout and stderr of a job");
    println!("  -s, --schedule              Enable scheduling mode (or list scheduled commands)");
    println!("  -r, --run <id>              Run a scheduled command immediately");
    println!("  --reschedule <id> <ts>      Change timespec for a scheduled command");
    println!("  --color[=WHEN]              Colorize output: 'always', 'never', or 'auto' (default: auto)");
    println!("  -n, --notify                Force desktop notification on job completion");
    println!("  --no-notify                 Disable desktop notification for job completion");
    println!("  -h, --help                  Show this help message");
    println!();
    println!("Schedule Options (with -s, --schedule, or 'schedule' alias):");
    println!("  -l, --list                  List all scheduled commands (default)");
    println!("  -k, --kill <id>             Remove a scheduled command");
    println!("  -d, --disable <id>          Disable a scheduled command");
    println!("  -e, --enable <id>           Enable a scheduled command");
    println!("  -r, --run <id>              Run a scheduled command immediately");
    println!("  --reschedule <id> <ts>      Change timespec for a scheduled command");
    println!("  --color[=WHEN]              Colorize output: 'always', 'never', or 'auto' (default: auto)");
    println!("  <timespec> <cmd> [args...]  Schedule a command for periodic or cron execution");
    println!();
    println!("Timespec Formats:");
    println!("  - Cron syntax:     \"0 12 * * *\", \"*/5 * * * *\", \"0 0 * * 1-5\"");
    println!("  - Human readable:  \"Wed 10 am\", \"daily at 10 am\", \"weekdays at 8:00 am\"");
    println!("  - Periodic:        \"every 5 hours\", \"every two minutes\", \"every 1 day\", \"5h\"");
}

fn print_schedule_help() {
    println!("schedule - command line asynchronous cron and periodic scheduler");
    println!();
    println!("Usage:");
    println!("  schedule [options]");
    println!("  schedule <timespec> <command> [args...]");
    println!();
    println!("Options:");
    println!("  -l, --list                  List all scheduled commands with last run and elapsed time");
    println!("  -k, --kill <id>             Remove a scheduled command by ID");
    println!("  -d, --disable <id>          Disable a scheduled command");
    println!("  -e, --enable <id>           Enable a scheduled command");
    println!("  -r, --run <id>              Run a scheduled command immediately");
    println!("  --reschedule <id> <ts>      Change timespec for a scheduled command");
    println!("  --color[=WHEN]              Colorize output: 'always', 'never', or 'auto' (default: auto)");
    println!("  -n, --notify                Force desktop notification when scheduled command finishes");
    println!("  --no-notify                 Disable desktop notification for scheduled command");
    println!("  -h, --help                  Show this help message");
    println!();
    println!("Timespec Formats:");
    println!("  - Cron syntax:     \"0 12 * * *\", \"*/5 * * * *\", \"0 0 * * 1-5\"");
    println!("  - Human readable:  \"Wed 10 am\", \"daily at 10 am\", \"weekdays at 8:00 am\"");
    println!("  - Periodic:        \"every 5 hours\", \"every two minutes\", \"every 1 day\", \"5h\"");
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

async fn handle_schedule_add(
    timespec: String,
    cmd: String,
    args: Vec<String>,
    notify: Option<bool>,
) {
    if let Err(e) = q::timespec::parse_timespec(&timespec) {
        eprintln!("Error: invalid timespec '{}': {}", timespec, e);
        std::process::exit(1);
    }

    let mut stream = connect_or_start_daemon().await;

    let work_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let env: Vec<(String, String)> = std::env::vars().collect();

    let req = Request::Schedule {
        timespec: timespec.clone(),
        cmd,
        args,
        work_dir,
        env,
        notify,
    };
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
        Response::Scheduled { schedule_id } => {
            println!("Scheduled command {} ('{}') successfully.", schedule_id, timespec);
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

async fn handle_schedule_list(should_color: bool) {
    let mut stream = connect_or_start_daemon().await;

    let req = Request::ScheduleList;
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
        Response::ScheduleList { schedules } => {
            print_schedules_table(&schedules, should_color);
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

async fn handle_schedule_kill(schedule_id: usize) {
    let mut stream = connect_or_start_daemon().await;

    let req = Request::ScheduleKill { schedule_id };
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
            println!("Scheduled command {} removed successfully.", schedule_id);
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

async fn handle_schedule_disable(schedule_id: usize) {
    let mut stream = connect_or_start_daemon().await;

    let req = Request::ScheduleDisable { schedule_id };
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
            println!("Scheduled command {} disabled successfully.", schedule_id);
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

async fn handle_schedule_enable(schedule_id: usize) {
    let mut stream = connect_or_start_daemon().await;

    let req = Request::ScheduleEnable { schedule_id };
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
            println!("Scheduled command {} enabled successfully.", schedule_id);
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

async fn handle_schedule_update(schedule_id: usize, timespec: String) {
    if let Err(e) = q::timespec::parse_timespec(&timespec) {
        eprintln!("Error: invalid timespec '{}': {}", timespec, e);
        std::process::exit(1);
    }

    let mut stream = connect_or_start_daemon().await;

    let req = Request::ScheduleUpdate { schedule_id, timespec: timespec.clone() };
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
            println!("Scheduled command {} rescheduled to '{}' successfully.", schedule_id, timespec);
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

async fn handle_schedule_run(schedule_id: usize) {
    let mut stream = connect_or_start_daemon().await;

    let req = Request::ScheduleRun { schedule_id };
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
            println!("Job {} queued for scheduled command {}.", job_id, schedule_id);
        }
        Response::Ok => {
            println!("Scheduled command {} queued successfully.", schedule_id);
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

async fn handle_list(should_color: bool, limit_override: Option<usize>) {
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
        Response::List { jobs } => {
            let config = load_config();
            let max_completed = limit_override.unwrap_or(config.max_completed_jobs_to_print);

            let (active_jobs, mut completed_jobs): (Vec<_>, Vec<_>) = jobs
                .into_iter()
                .partition(|j| j.status == "running" || j.status == "queued");

            completed_jobs.sort_by_key(|j| j.id);
            if completed_jobs.len() > max_completed {
                let drop_count = completed_jobs.len() - max_completed;
                completed_jobs.drain(0..drop_count);
            }

            let mut final_jobs = active_jobs;
            final_jobs.extend(completed_jobs);

            if final_jobs.is_empty() {
                println!("No jobs in queue.");
                return;
            }

            // Sort jobs: Running (priority 0), Queued (priority 1), others (priority 2), then by ID ascending
            final_jobs.sort_by(|a, b| {
                let prio_a = get_status_priority(&a.status);
                let prio_b = get_status_priority(&b.status);
                if prio_a != prio_b {
                    prio_a.cmp(&prio_b)
                } else {
                    a.id.cmp(&b.id)
                }
            });

            print_jobs_table(&final_jobs, should_color);
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

fn print_jobs_table(jobs: &[JobInfoShort], should_color: bool) {
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
        let status_display = if should_color {
            let color = match JobStatus::from_str(&status) {
                JobStatus::Running => "\x1b[33m",
                JobStatus::Completed { exit_code: 0 } => "\x1b[32m",
                JobStatus::Completed { .. } | JobStatus::Failed { .. } | JobStatus::Cancelled => "\x1b[31m",
                _ => "",
            };
            if color.is_empty() {
                format!("{:<status_width$}", status, status_width = max_status_len)
            } else {
                format!("{}{:<status_width$}\x1b[0m", color, status, status_width = max_status_len)
            }
        } else {
            format!("{:<status_width$}", status, status_width = max_status_len)
        };

        println!(
            "{:<id_width$}  {}  {:<pid_width$}  {:<start_width$}  {:<time_width$}  {}",
            id, status_display, pid_str, start_str, duration_str, cmd,
            id_width = max_id_len,
            pid_width = max_pid_len,
            start_width = max_start_len,
            time_width = max_time_len
        );
    }
}

fn get_schedule_last_run_color(s: &ScheduleInfoShort) -> &'static str {
    if s.is_running {
        "\x1b[33m"
    } else if s.last_run.is_none() {
        ""
    } else if let Some(ref status) = s.last_status {
        if status == "exit 0" {
            "\x1b[32m"
        } else if status.starts_with("exit ") || status == "failed" || status == "cancelled" {
            "\x1b[31m"
        } else if status == "running" {
            "\x1b[33m"
        } else {
            ""
        }
    } else {
        ""
    }
}

fn print_schedules_table(schedules: &[ScheduleInfoShort], should_color: bool) {
    if schedules.is_empty() {
        println!("No scheduled commands.");
        return;
    }

    let mut max_id_len = 2; // "ID"
    let mut max_timespec_len = 8; // "TIMESPEC"
    let mut max_last_run_len = 8; // "LAST RUN"
    let mut max_elapsed_len = 7; // "ELAPSED"
    let mut max_next_run_len = 8; // "NEXT RUN"

    let mut formatted = Vec::new();
    let now_utc = chrono::Utc::now();

    for s in schedules {
        let last_run_str = if let Some(ref lr) = s.last_run {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(lr) {
                let dt_local = dt.with_timezone(&chrono::Local);
                let formatted_dt = dt_local.format("%Y-%m-%d %H:%M:%S").to_string();

                let suffix = if s.is_running {
                    " (running)".to_string()
                } else if let Some(ref status) = s.last_status {
                    if status == "running" {
                        " (running)".to_string()
                    } else if status == "failed" {
                        " (failed)".to_string()
                    } else if status == "cancelled" {
                        " (cancelled)".to_string()
                    } else if status.starts_with("exit ") {
                        format!(" ({})", status)
                    } else {
                        "".to_string()
                    }
                } else {
                    "".to_string()
                };

                format!("{}{}", formatted_dt, suffix)
            } else {
                "--".to_string()
            }
        } else {
            "--".to_string()
        };

        let elapsed_str = if let Some(ref lr) = s.last_run {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(lr) {
                let dt_utc = dt.with_timezone(&chrono::Utc);
                let diff = now_utc.signed_duration_since(dt_utc);
                format_duration(diff.num_seconds())
            } else {
                "--".to_string()
            }
        } else {
            "never".to_string()
        };

        let next_run_str = if let Some(ref nr) = s.next_run {
            if nr == "DISABLED" {
                "DISABLED".to_string()
            } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(nr) {
                let dt_local = dt.with_timezone(&chrono::Local);
                let formatted_dt = dt_local.format("%Y-%m-%d %H:%M:%S").to_string();

                let dt_utc = dt.with_timezone(&chrono::Utc);
                let diff_secs = dt_utc.signed_duration_since(now_utc).num_seconds();
                let suffix = if diff_secs <= 0 {
                    "(due)".to_string()
                } else {
                    format!("(in {})", format_relative_duration(diff_secs))
                };

                format!("{} {}", formatted_dt, suffix)
            } else {
                nr.clone()
            }
        } else {
            "--".to_string()
        };

        max_id_len = max_id_len.max(s.id.to_string().len());
        max_timespec_len = max_timespec_len.max(s.timespec.len());
        max_last_run_len = max_last_run_len.max(last_run_str.len());
        max_elapsed_len = max_elapsed_len.max(elapsed_str.len());
        max_next_run_len = max_next_run_len.max(next_run_str.len());

        let color = get_schedule_last_run_color(s);
        formatted.push((
            color,
            s.id,
            s.timespec.clone(),
            last_run_str,
            elapsed_str,
            next_run_str,
            s.cmd.clone(),
        ));
    }

    println!(
        "{:<id_w$}  {:<ts_w$}  {:<lr_w$}  {:<el_w$}  {:<nr_w$}  {}",
        "ID", "TIMESPEC", "LAST RUN", "ELAPSED", "NEXT RUN", "COMMAND",
        id_w = max_id_len,
        ts_w = max_timespec_len,
        lr_w = max_last_run_len,
        el_w = max_elapsed_len,
        nr_w = max_next_run_len,
    );
    println!(
        "{:-<id_w$}--{:-<ts_w$}--{:-<lr_w$}--{:-<el_w$}--{:-<nr_w$}--{:-<20}",
        "", "", "", "", "", "",
        id_w = max_id_len,
        ts_w = max_timespec_len,
        lr_w = max_last_run_len,
        el_w = max_elapsed_len,
        nr_w = max_next_run_len,
    );

    for (color, id, ts, lr, el, nr, cmd) in formatted {
        let lr_display = if should_color && !color.is_empty() {
            format!("{}{:<lr_w$}\x1b[0m", color, lr, lr_w = max_last_run_len)
        } else {
            format!("{:<lr_w$}", lr, lr_w = max_last_run_len)
        };

        let nr_display = if should_color && nr == "DISABLED" {
            format!("\x1b[31m{:<nr_w$}\x1b[0m", nr, nr_w = max_next_run_len)
        } else {
            format!("{:<nr_w$}", nr, nr_w = max_next_run_len)
        };

        println!(
            "{:<id_w$}  {:<ts_w$}  {}  {:<el_w$}  {}  {}",
            id, ts, lr_display, el, nr_display, cmd,
            id_w = max_id_len,
            ts_w = max_timespec_len,
            el_w = max_elapsed_len,
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
