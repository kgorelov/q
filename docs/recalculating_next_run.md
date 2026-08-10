  I have updated the schedule re-enable and update logic to re-calculate the NEXT RUN time forward from the moment the command is re-enabled or rescheduled.
  ──────
  ### Key Adjustments

  1. Forward Recalculation on Re-Enable & Reschedule:
      • Added enabled_at: Option<String> in lib.rs's ScheduleSpec.
      • When a scheduled job is enabled (q --schedule --enable <id> / schedule -e <id>) or rescheduled (q -r <id> <timespec>), enabled_at is set to the current local
      timestamp.
      • The daemon's queue manager and schedule listing in qdaemon.rs use max(last_run, enabled_at) as the baseline reference.
  2. Benefits:
      • No accidental immediate execution: Periods during which the command was disabled are ignored, and the command will not fire immediately upon being re-enabled.
      • Preserved historical execution record: The LAST RUN column in schedule / q -s continues to accurately reflect the previous real execution time (or -- if it
      hasn't run yet), while NEXT RUN reflects the newly computed forward schedule.
