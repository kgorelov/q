A simple command line tool to queue long commands and execute them in a process pool.

## Architecture / design

qdaemon - daemon that controls execution of the commands from the queue.

q - command line tool that allows users putting command in the queue, listing jobs, killing jobs, getting jobs statuses, getting jobs logs.

Language of choise for imlementing both, the daemon and the command line tool is Rust.
The daemon is run by a user, either explicitely (systemd, bashrc) or by the command line tool (q) on the first run if it doesn't find the daemon to communicate to.

Each user may have a config file (~/.config/q/q.conf) which will define how many jobs are allowed to run in parallel.
The daemon spawns jobs and keeps track of the running jobs, waiting for their pids. 
If the daemon dies, it will re-read the status from disk, check if jobs are still runing.
If the job dies before the daemon respawns (computer turned off) the status must indicate a failed job with no status.

The queue must be implemented on disk (/var/spool/q or user based ~/.q/spool).
Each job gets a directory where will be files:
  pid - hold running process PID
  status - current job status, including exit code when the process dies
  stdout - process's stdout collected here
  stderr - process's stderr collected here

The main q daemon config file will define the retention policy for the spool directory, how many jobs to keep, when to rotate completed jobs directories.

## Usage examples

### Queue a command
q command [args]

Puts the command to the queue.

### List all command (queued and running)

q --list
q -l

List command shows a table of jobs: running on top, queued, then completed, with their statuses.

### Kill a running command
q --kill <jobid>
q -k <jobid>

Kills a running job or cancels a queued job. In the latter case the job moves to completed with a corresponding status.


### Pring logs
q --logs <jobid>

Prints stdout and stderr of the job. The tricky part is making the output interleaved like it would be if the process wrote everything to a terminal. Let's leave this problem aside for now.

