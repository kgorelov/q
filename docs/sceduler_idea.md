## Scheduling feature

This new feature implements an asynchronous cron in userland. As with the q command there will be two parts: the daemon and the command line tool. The already existing qdaemon will serve two purposes: it will continue to control life-cycle of queued jobs, and will now serve as a cron daemon. Whenever it's time to execute a scheduled command, it is put into the queue. The command line part will also remain the same, a new options will be added to the 'q' command: '--schedule' which turns on scheduling mode.

The scheduler is async. It is implied that the computer may have been turned off when it was the right time to run the command. It's ok, after the daemon starts it will check what command must have been run and will catch up. If for instance a backup is scheduled for Wednesdays and now is Thursday and more than a day has passed since last run, it means it's time to run.


### Usage examples
The 'q' command will receive a new long option '--schedule' and an equivalent short option '-s'.
schedule is an alias for q --schedule

#### schedule a command
q --schedule <timespec> command [args]

Schedule command execution.
The timespec is the trickiest part, this may be:
  - a cron like string
  - a human readable string like "Wed 10 am"
  - a period, say "every 5 hours"


#### List all scheduled commands
q --schedule
q --schedule --list
q --schedule -l

List scheduled commands along with information when the command was run the last time and how much time elapsed since the last run.

#### Remove a scheduled command
schedule --kill <id>

Removes the scheduled command from the list.
