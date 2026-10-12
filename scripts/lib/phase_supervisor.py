"""Keep a private process-group leader alive until the qualification driver reaps it.

The command is reaped before the atomic completion marker. Keeping this leader
alive prevents its process-group ID from being reused before group cleanup.
"""
import json
import pathlib
import subprocess
import sys


def main():
    completion = pathlib.Path(sys.argv[1])
    result = {}
    try:
        child = subprocess.Popen(sys.argv[2:], stdin=subprocess.DEVNULL)
        result['exit_code'] = child.wait()
    except OSError as error:
        result.update(exit_code=None, error=type(error).__name__, detail=str(error))
    temporary = completion.with_suffix('.tmp')
    temporary.write_text(json.dumps(result) + '\n')
    temporary.replace(completion)
    # No command or descendant inherits this pipe. The parent holds it open
    # until group cleanup; normal command completion cannot release our PID.
    sys.stdin.buffer.read(1)


if __name__ == '__main__':
    main()
