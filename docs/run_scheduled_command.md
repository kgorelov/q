  ### Supported Invocations

  • schedule --run <id>
  • schedule -r <id>
  • q --schedule --run <id>
  • q --schedule -r <id>
  • q -s --run <id>
  • q -s -r <id>
  • q --run <id>
  • q -r <id>

  (Note: If -r is given a single <id> argument, it triggers an immediate run; if given <id> <timespec>, it reschedules the command).
  ──────
  ### Implementation Details

  1. Protocol & Daemon Handling (lib.rs & qdaemon.rs):
      • Added Request::ScheduleRun { schedule_id }.
      • The daemon immediately enqueues a new execution of the scheduled command into ~/.q/spool/ and returns the newly queued job_id.
      • Updates the schedule's last_run timestamp to now, advancing subsequent scheduled occurrences accordingly.
  2. Client CLI (q.rs):
      • Added --run and -r <id> command parsing.
      • Outputs: Job <job_id> queued for scheduled command <schedule_id>.
  3. Documentation:
      • Updated q.1 and README.md with usage examples and descriptions of --run / -r <id>.

  ──────
  ### Example

    # Run scheduled command #1 immediately:
    schedule --run 1
    # or
    q -r 1

  Output:

    Job 97 queued for scheduled command 1.
