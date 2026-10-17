use std::fs;
use std::process::Command;
use std::time::Duration;

#[test]
fn test_daemon_logging_and_rotation_integration() {
    let test_dir = std::env::temp_dir().join(format!("q_e2e_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&test_dir);
    let q_dir = test_dir.join(".q");
    let _ = fs::create_dir_all(&q_dir);

    // Create a q.conf with small log rotation size limit
    let conf_path = q_dir.join("q.conf");
    fs::write(
        &conf_path,
        r#"
max_parallel_jobs = 2
max_completed_jobs_to_keep = 10
enable_notifications = false
max_log_files = 10
max_log_file_size = "400b"
"#,
    )
    .unwrap();

    let target_dir = std::env::current_dir().unwrap().join("target").join("release");
    let exe_suffix = std::env::consts::EXE_SUFFIX;
    let qdaemon_bin = target_dir.join(format!("qdaemon{}", exe_suffix));
    let q_bin = target_dir.join(format!("q{}", exe_suffix));

    assert!(qdaemon_bin.exists(), "qdaemon binary not found");
    assert!(q_bin.exists(), "q binary not found");

    // Launch daemon with HOME / USERPROFILE set to test_dir
    let mut daemon_child = Command::new(&qdaemon_bin)
        .env("HOME", &test_dir)
        .env("USERPROFILE", &test_dir)
        .spawn()
        .expect("Failed to start qdaemon");

    // Wait for daemon to create socket/port and write initial logs
    let log_path = q_dir.join("qdaemon.log");
    #[cfg(unix)]
    let ready_path = q_dir.join("q.sock");
    #[cfg(windows)]
    let ready_path = q_dir.join("q.port");

    for _ in 0..50 {
        if ready_path.exists()
            && log_path.exists()
            && fs::metadata(&log_path).map(|m| m.len() > 0).unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(log_path.exists(), "qdaemon.log was not created");

    // Queue a job via `q`
    #[cfg(windows)]
    let (test_cmd, test_prefix_args): (&str, &[&str]) = ("cmd", &["/C", "echo"]);
    #[cfg(not(windows))]
    let (test_cmd, test_prefix_args): (&str, &[&str]) = ("echo", &[]);

    let output = Command::new(&q_bin)
        .env("HOME", &test_dir)
        .env("USERPROFILE", &test_dir)
        .arg(test_cmd)
        .args(test_prefix_args)
        .arg("Hello Daemon Log")
        .output()
        .expect("Failed to run q");

    assert!(output.status.success());

    // Schedule a job via `q`
    let sched_output = Command::new(&q_bin)
        .env("HOME", &test_dir)
        .env("USERPROFILE", &test_dir)
        .arg("--schedule")
        .arg("every 1 hour")
        .arg(test_cmd)
        .args(test_prefix_args)
        .arg("scheduled task")
        .output()
        .expect("Failed to schedule job");

    assert!(sched_output.status.success());

    // Queue enough jobs to cause log rotation (over 400 bytes)
    for i in 0..8 {
        let _ = Command::new(&q_bin)
            .env("HOME", &test_dir)
            .env("USERPROFILE", &test_dir)
            .arg(test_cmd)
            .args(test_prefix_args)
            .arg(format!("Logging job payload message number {}", i))
            .output();
        std::thread::sleep(Duration::from_millis(100));
    }

    // Give daemon time to process and rotate
    std::thread::sleep(Duration::from_millis(500));

    // Check that log file exists and rotated file .1 exists
    let log_content = fs::read_to_string(&log_path).unwrap_or_default();
    let rot1_path = q_dir.join("qdaemon.log.1");

    assert!(
        rot1_path.exists(),
        "Log rotation (qdaemon.log.1) should have occurred"
    );

    let mut all_logs = log_content;
    for idx in 1..=10 {
        let rot_path = q_dir.join(format!("qdaemon.log.{}", idx));
        if rot_path.exists() {
            if let Ok(content) = fs::read_to_string(&rot_path) {
                all_logs.push('\n');
                all_logs.push_str(&content);
            }
        }
    }

    assert!(
        all_logs.contains("qdaemon"),
        "Log should contain daemon startup. Actual logs:\n{}",
        all_logs
    );
    assert!(
        all_logs.contains("Job #1"),
        "Log should contain Job #1. Actual logs:\n{}",
        all_logs
    );
    assert!(
        all_logs.contains("Schedule #1"),
        "Log should contain Schedule #1. Actual logs:\n{}",
        all_logs
    );

    // Terminate daemon
    let _ = daemon_child.kill();
    let _ = daemon_child.wait();

    let _ = fs::remove_dir_all(&test_dir);
}
