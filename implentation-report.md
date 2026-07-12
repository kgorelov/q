# Implementation Report: q and qdaemon

I have implemented the command queue tools `q` and `qdaemon` in Rust as described in `idea.md`.

Here is a summary of the implementation and layout of the project:

## 1. Project Configuration & Shared Library
* **Cargo.toml**: Configured with standard dependencies (`tokio` for async networking/subprocesses, `serde` and `serde_json` for serialization/IPC protocol, `toml` for parsing configuration, `dirs` for cross-platform user directories, and `libc` for signals/session detachment).
* **src/lib.rs**: Contains shared logic, including:
  * Directory & file path resolution: `~/.q/spool` for jobs, `~/.q/q.sock` for the UNIX domain socket, `~/.q/qdaemon.pid` for the daemon process locking, and `~/.config/q/q.conf` for the configuration file.
  * `Config`: Represents daemon execution parameters (`max_parallel_jobs` and `max_completed_jobs`).
  * `JobStatus`: Enumerates states (`Queued`, `Running`, `Completed { exit_code }`, `Failed { error }`, `Cancelled`).
  * `Request` & `Response`: Define the JSON-over-UDS line IPC protocol.

## 2. The Daemon: `qdaemon`
Implemented in **src/bin/qdaemon.rs**:
* **run_daemon**: Establishes a UNIX Domain Socket server at `~/.q/q.sock`, locks running PIDs, and boots the queue execution manager.
* **run_queue_manager**: Monitored via standard `tokio::sync::mpsc` trigger channels. It parses user configurations on-the-fly, processes queued commands, enforces the parallel run limit, and prunes older finished jobs according to the spool retention policy.
* **Startup Recovery**: On initialization, `qdaemon` inspects the spool directories. It recovers and resumes tracking active orphaned child processes, and marks crashed runs as `failed` if the machine restarted while they were running.
* **run_worker** (`qdaemon --worker <jobid>`): Solves the daemon crash/restart vulnerability. Spawning workers in detached mode guarantees stdout/stderr redirects (`~/.q/spool/<id>/stdout` and `~/.q/spool/<id>/stderr`) are fully persisted and final exit codes written to disk even if `qdaemon` restarts during a run.

## 3. The Client CLI: `q`
Implemented in **src/bin/q.rs**:
* **connect_or_start_daemon**: Attempts to connect to `q.sock`. If no server is listening, it spawns `qdaemon` using Unix `pre_exec` with `libc::setsid()` to detach it into its own session.
* **Queueing**: Automatically captures the caller's working directory (`std::env::current_dir`) and environment variables (`std::env::vars`) to execute the background command in the exact same environment.
* **Listing**: Formats the active and completed queue into a dynamically padded ASCII table, sorted by importance (Running > Queued > Completed/Failed/Cancelled).
* **Killing**: Communicates with the daemon to abort queued commands or terminate running processes by signaling the worker and child process groups.
* **Logs**: Direct log piping. Outputs the job's stdout directly to standard out and stderr to standard error, preserving shell pipes/redirects.

## Verification & Testing
I successfully built and verified the program:
1. **Auto-spawning & Queuing**: Running `./target/debug/q sleep 2` correctly started the background daemon and registered Job 1.
2. **Parallel execution & Retention**: Configured `max_parallel_jobs = 3` and `max_completed_jobs = 5` in `~/.config/q/q.conf`. Enqueued four parallel jobs: three ran concurrently, the fourth remained queued, and older logs were rotated out correctly.
3. **Killing & Cancelling**: Tested killing running processes (moving to `cancelled`) and Cancelling queued items successfully.
4. **Log Redirection**: Checked log output using `./target/debug/q --logs <id>` to verify that outputs on both standard descriptors were printed correctly.
