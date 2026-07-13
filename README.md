# q - Command Line Queue and Execution Utility

`q` is a lightweight, zero-dependency (other than standard library, tokio, and chrono) command-line utility for Linux that lets you queue commands and execute them sequentially or in parallel in the background. It is managed by a background daemon (`qdaemon`) which communicates with the client via a Unix domain socket.

If the daemon is not running when you interact with `q`, the client will automatically start it in the background.

---

## Features

- **Background Queueing**: Queue commands to run asynchronously in the background.
- **Parallel Execution**: Execute multiple jobs concurrently, up to a configurable limit.
- **Start Time & Duration Tracking**: Keep track of exactly when a job started and how long it has been running (or took to complete).
- **Execution Log Capture**: Access `stdout` and `stderr` logs for any job at any time.
- **Graceful Control**: Kill running jobs or cancel queued jobs.
- **Configurable Retention**: Auto-cleanup of old jobs according to a retention policy.

---

## Installation & Compilation

Ensure you have Rust and Cargo installed, then clone the repository and run:

```bash
cargo build --release
```

This compiles two binaries in `target/release/`:
- `q` (the client interface)
- `qdaemon` (the background queue worker)

You can copy these binaries into your `PATH` (e.g., `/usr/local/bin/`).

---

## Command Usage

```bash
q [options]
q <command> [args...]
```

### Options

| Option | Description |
|---|---|
| `-l`, `--list` | Lists all queued, running, and completed jobs (default behavior). |
| `-k`, `--kill <id>` | Kills a running job or cancels a queued job. |
| `-L`, `--logs <id>` | Prints the captured stdout and stderr logs for a job. |
| `-h`, `--help` | Prints the help message. |

### Examples

1. **Queue a job**:
   ```bash
   q sleep 10
   q cargo build --release
   ```

2. **List jobs**:
   ```bash
   q
   ```
   *Output format:*
   ```text
   JOB ID  STATUS          PID    START TIME           TIME  COMMAND
   ------------------------------------------------------------------------------
   18      running         24367  2026-07-13 23:21:58  4s    sleep 10
   16      completed (0)          2026-07-13 23:21:22  2s    sleep 3
   17      completed (0)          2026-07-13 23:21:36  5s    sleep 5
   ```

3. **View logs**:
   ```bash
   q --logs 16
   ```

4. **Kill a job**:
   ```bash
   q --kill 18
   ```

---

## Configuration

`qdaemon` reads its configuration from a TOML file. It searches for a configuration file in the following order:
1. `$XDG_CONFIG_HOME/q/q.conf` (typically `~/.config/q/q.conf`)
2. `~/.q/q.conf`

If no configuration file is found, it uses default values.

### Configuration Options

```toml
# Configuration for qdaemon

# Maximum number of jobs allowed to run in parallel
max_parallel_jobs = 3

# Maximum number of finished jobs to keep in history before deleting old records
max_completed_jobs = 50
```

---

## How It Works

- **Auto-Daemon Start**: The `q` client automatically looks for a socket at `~/.q/q.sock`. If the socket is missing or unresponsive, the client spawns `qdaemon` as a detached process and waits for it to start listening.
- **State Storage**: The daemon stores job information and command outputs in `~/.q/spool/<job_id>/`.
  - `spec.json`: Command details (arguments, directory, environment).
  - `status`: Current state (`queued`, `running`, `completed <exit_code>`, `failed: <error>`, `cancelled`).
  - `start_time` / `end_time`: RFC3339 timestamps for tracking durations.
  - `stdout` / `stderr`: Captured process streams.
  - `pid` / `worker_pid`: PIDs for tracking and killing.

---

## Man Page

To view the manual page for `q`:
```bash
man ./q.1
```
