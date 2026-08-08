# q - Command Line Queue, Execution & Scheduling Utility

`q` is a lightweight, zero-dependency (other than standard library, tokio, and chrono) command-line utility for Linux that lets you queue commands, execute them in parallel, and schedule them with an asynchronous userland cron daemon (`qdaemon`).

If the daemon is not running when you interact with `q` or `schedule`, the client will automatically start it in the background.

---

## Features

- **Background Queueing**: Queue commands to run asynchronously in the background.
- **Parallel Execution**: Execute multiple jobs concurrently, up to a configurable limit.
- **Asynchronous Userland Cron & Scheduler**: Schedule commands with cron expressions, human-readable times (`Wed 10 am`, `weekdays at 8:00 am`), or periodic intervals (`every 5 hours`, `30m`).
- **Power-Off Catch-Up**: Asynchronous catch-up ensures scheduled jobs that were missed while the machine was turned off or asleep run when the daemon resumes.
- **Start Time & Duration Tracking**: Keep track of start time, duration, and elapsed time since last run.
- **Execution Log Capture**: Access `stdout` and `stderr` logs for any job at any time.
- **Graceful Control**: Kill running jobs, cancel queued jobs, or remove scheduled commands.
- **Configurable Retention**: Auto-cleanup of old jobs according to a retention policy.

---

## Installation & Compilation

Ensure you have Rust and Cargo installed, then clone the repository and run:

```bash
cargo build --release
```

This compiles two binaries in `target/release/`:
- `q` (the client interface and `schedule` alias)
- `qdaemon` (the background queue worker and cron daemon)

You can install them to `/usr/local/bin` using:
```bash
sudo make install
```

---

## Command Usage

### Queue & Job Execution
```bash
q [options]
q [notification-options] <command> [args...]
```

### Scheduling Commands
```bash
q -s, --schedule [schedule-options]
q -s, --schedule <timespec> <command> [args...]

# 'schedule' is an alias for 'q --schedule':
schedule [schedule-options]
schedule <timespec> <command> [args...]
```

### Options

| Option | Description |
|---|---|
| `-l`, `--list` | Lists all queued, running, and completed jobs (default behavior). |
| `-k`, `--kill <id>` | Kills a running job or cancels a queued job. |
| `-L`, `--logs <id>` | Prints the captured stdout and stderr logs for a job. |
| `-s`, `--schedule` | Enables scheduling mode. |
| `-n`, `--notify` | Force desktop notification on job completion. |
| `--no-notify` | Disable desktop notification for job completion. |
| `-h`, `--help` | Prints the help message. |

### Schedule Options (with `-s`, `--schedule`, or `schedule` command)

| Option | Description |
|---|---|
| `-l`, `--list` | Lists all scheduled commands, last run time, and elapsed time (default). |
| `-k`, `--kill <id>` | Removes a scheduled command by ID. |
| `<timespec> <cmd> [args...]` | Schedules a command for periodic or cron execution. |

---

## Timespec Formats

The scheduler supports flexible timespec expressions:

1. **Cron-like Strings**:
   ```bash
   q -s "0 12 * * *" /usr/local/bin/backup.sh
   q -s "*/15 * * * *" check_health.sh
   schedule "0 0 * * 1-5" weekday_job.sh
   schedule "0 10 * * Wed" weekly_report.sh
   ```

2. **Human-Readable Times**:
   ```bash
   schedule "Wed 10 am" backup.sh
   schedule "Wednesday 10:30 pm" sync_data.sh
   schedule "weekdays at 8:00 am" daily_standup.sh
   schedule "daily at 14:00" cleanup.sh
   schedule "Mon,Wed,Fri 9am" team_sync.sh
   ```

3. **Periodic Intervals**:
   ```bash
   schedule "every 5 hours" backup.sh
   schedule "every 30 minutes" sync_metrics.sh
   schedule "every 1 day" report.sh
   schedule "5h" quick_sync.sh
   schedule "30m" ping_servers.sh
   ```

---

## Examples

1. **Queue a job**:
   ```bash
   q sleep 10
   q cargo build --release
   ```

2. **List queued and running jobs**:
   ```bash
   q
   ```
   *Output:*
   ```text
   JOB ID  STATUS          PID    START TIME           TIME  COMMAND
   ------------------------------------------------------------------------------
   18      running         24367  2026-07-13 23:21:58  4s    sleep 10
   16      completed (0)          2026-07-13 23:21:22  2s    sleep 3
   17      completed (0)          2026-07-13 23:21:36  5s    sleep 5
   ```

3. **Schedule a command**:
   ```bash
   schedule "Wed 10 am" backup.sh --all
   schedule "every 5 hours" sync_data.sh
   ```

4. **List scheduled commands**:
   ```bash
   schedule
   # or
   q --schedule --list
   ```
   *Output:*
   ```text
   ID  TIMESPEC       LAST RUN             ELAPSED  NEXT RUN             COMMAND
   --------------------------------------------------------------------------------
   1   Wed 10 am      2026-08-05 10:00:00  3d 1h    2026-08-12 10:00:00  backup.sh --all
   2   every 5 hours  2026-08-08 07:00:00  4h 50m   2026-08-08 12:00:00  sync_data.sh
   ```

5. **Remove a scheduled command**:
   ```bash
   schedule --kill 1
   # or
   q --schedule -k 1
   ```

6. **View logs & Kill jobs**:
   ```bash
   q --logs 16
   q --kill 18
   ```

---

## Configuration

`qdaemon` reads its configuration from a TOML file:
1. `$XDG_CONFIG_HOME/q/q.conf` (typically `~/.config/q/q.conf`)
2. `~/.q/q.conf`

```toml
# Configuration for qdaemon

# Maximum number of jobs allowed to run in parallel
max_parallel_jobs = 3

# Maximum number of finished jobs to keep in history before deleting old records
max_completed_jobs = 50

# Enable desktop notifications for long-running commands (default: true)
enable_notifications = true

# Minimum duration in seconds a command must run before triggering a completion notification (default: 10)
min_notify_duration_secs = 10
```

---

## How It Works

- **Auto-Daemon Start**: The client automatically starts `qdaemon` if it is not already running.
- **State Storage**:
  - `~/.q/spool/<job_id>/`: Metadata (`spec.json`, `status`, `start_time`, `end_time`) and captured process streams (`stdout`, `stderr`).
  - `~/.q/schedules/<schedule_id>/`: Schedule metadata (`spec.json`), tracking timestamps (`last_run`), and last triggered job IDs (`last_job_id`).
- **Asynchronous Catch-Up**: When `qdaemon` starts or ticks, it computes whether any scheduled run was missed and automatically enqueues it.

---

## Man Page

To view the manual page:
```bash
man ./q.1
```
