## Scehduled commands statuses

Currently, in the scheduled commands output there's the last run time.
Here's an example:

$ ./schedule
ID  TIMESPEC           LAST RUN             ELAPSED  NEXT RUN             COMMAND
----------------------------------------------------------------------------------------------
1   every two minutes  2026-08-08 20:00:29  1m 46s   2026-08-08 20:02:29  /home/kgorelov/bin/test_scheduler.sh

What I would like to add is the last result.
