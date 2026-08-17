## qdaemon logginig

Problem: qdaemon desn't log anything => it's hard to debug, hard to tell what was launched when and why...

Solution:
  - Log everything under ~/.q/qdaemon.log
  - Rotate the logs: ~/.q/qdaemon.log -> ~/.q/qdaemon.log.1 -> ~/.q/qdaemon.log.2 ...
  - Do not depend on logrotate (especially since we have a windows version)
    Implement a simple log rotation mechanism right inside qdaemon.
  - Control the maximum number of log files to keep with max_log_files config param
  - Control when to rotate by the following config params:
    - max_log_file_age - rotate if the log file is older than that. The may be in hours, days, weeks, years. Example "1w".
    - max_log_file_size - rotate if the file is bigger than this. The size may be in bytes, kb, mb, gb. Example "200mb"
