  I have implemented the Rescheduling feature for scheduled commands.
  ──────
  ### Supported Invocations

  You can change the timespec of any existing scheduled command using any of the following formats:

  • q --reschedule <jobid> <timespec>
  • q -r <jobid> <timespec>
  • q --schedule --reschedule <jobid> <timespec>
  • q --schedule -r <jobid> <timespec>
  • q -s --reschedule <jobid> <timespec>
  • q -s -r <jobid> <timespec>
  • schedule --reschedule <jobid> <timespec>
  • schedule -r <jobid> <timespec>

  (The --reschedule / -r flag implies --schedule, referencing the ID of a scheduled job).
  ──────
  ### Implementation Details

  1. Daemon & Protocol (lib.rs & qdaemon.rs):
      • Added Request::ScheduleUpdate { schedule_id, timespec }.
      • Validates the new timespec and updates timespec & parsed in spec.json atomically.
      • Recalculates next occurrence based on the new timespec.
  2. Client CLI (q.rs):
      • Added -r / --reschedule <jobid> <timespec> argument parsing in both q and schedule.
      • Validates the timespec on the client side before sending the request.
  3. Documentation:
      • Updated q.1 and README.md with usage examples and descriptions of --reschedule / -r.

  ──────
  ### Example

    # Reschedule job #2 to every 2 hours:
    q -r 2 "every 2 hours"
    # or
    schedule --reschedule 2 "every 2 hours"

  Output:

    Scheduled command 2 rescheduled to 'every 2 hours' successfully.
