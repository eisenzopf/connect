#!/usr/bin/env python3
"""An inherited pipe and loopback listener expose actual owned-child leaks."""
import json
import os
import socket
import sys
import time
from pathlib import Path

with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    path = Path(sys.argv[1])
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps({
        "pid": os.getpid(), "group": os.getpgrp(), "parent": os.getppid(),
        "port": listener.getsockname()[1],
    }))
    temporary.replace(path)
    # A fixture fails closed even if the implementation under test leaks it.
    time.sleep(60)
