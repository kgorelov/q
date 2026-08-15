  I have implemented the qdaemon logging and log rotation feature as described in daemon_logging_idea.md.

  ### Summary of Implementation

  1. Logging & Log Rotation Module (logging.rs):
      • Log Location: Logs all events to ~/.q/qdaemon.log via lib.rs.
      • Internal Rotation Engine: Implements logging.rs without external tools like logrotate (compatible with Unix & Windows):
          • Shifts existing logs (qdaemon.log → qdaemon.log.1 → qdaemon.log.2 … up to max_log_files).
          • Prunes rotated logs exceeding max_log_files.
      • Size-based Rotation: Parses units including bytes, kb, mb, gb, tb (case-insensitive, e.g. "200mb", "500kb", "1gb").
      • Age-based Rotation: Parses units including seconds, minutes, hours, days, weeks, months, years (e.g. "1w", "24h", "30m", "1y").
      • Thread & Async Safe: logging.rs provides atomic log formatting, flushing, and automatic rotation.
  2. Configuration Support (lib.rs):
      • Added configuration fields to lib.rs:
          • max_log_files (default: 5)
          • max_log_file_age (e.g. "1w")
          • max_log_file_size (e.g. "200mb")
      • Added custom deserializer lib.rs supporting both string format and numeric values in q.conf.
  3. Daemon & Worker Instrumentation (qdaemon.rs):
      • Logged daemon startup, shutdown, and orphan recovery.
      • Logged client RPC requests (Queue, Kill, Schedule, Enable/Disable, Reschedule, Run).
      • Logged worker lifecycle, execution outcomes, duration, and error diagnostics.
      • Integrated periodic rotation checks in the queue manager ticker.
  4. Documentation & Tests:
      • Updated README.md and man page q.1.
      • Added unit tests in logging.rs and lib.rs.
      • Added an end-to-end integration test in integration_tests.rs.
