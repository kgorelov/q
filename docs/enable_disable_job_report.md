  ### Key Additions

  1. Disabling Scheduled Commands:
      • Supported CLI invocations:
          • q --schedule --disable <jobid>
          • q --schedule -d <jobid>
          • q -s --disable <jobid>
          • q -s -d <jobid>
          • schedule --disable <jobid>
          • schedule -d <jobid>
      • When disabled, the scheduler skips execution of this task.
      • When listing schedules via schedule or q -s, the NEXT RUN column displays DISABLED.
  2. Enabling Scheduled Commands:
      • Supported CLI invocations:
          • q --schedule --enable <jobid>
          • q --schedule -e <jobid>
          • q -s --enable <jobid>
          • q -s -e <jobid>
          • schedule --enable <jobid>
          • schedule -e <jobid>
      • Restores the scheduled task to active state, recalculating the next occurrence in the NEXT RUN column and resuming execution checks.
  3. IPC Protocol & State Persistence:
      • Extended lib.rs with Request::ScheduleDisable and Request::ScheduleEnable.
      • Added enabled: bool (defaulting to true for backwards compatibility) in ScheduleSpec.
      • Handled atomic enable/disable state updates and list formatting in qdaemon.rs.
      • Added CLI parsing and handler routines in q.rs.
  4. Documentation:
      • Updated q.1 and README.md with usage examples and descriptions of -d/--disable and -e/--enable.


  The release builds and test suites have completed successfully:

  • Binaries built in target/release/:
      • q
      • qdaemon
      • schedule


  The enable/disable commands (-d/--disable and -e/--enable) and DISABLED status reporting in NEXT RUN are ready to use.
