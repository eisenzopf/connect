#!/usr/bin/env python3
"""Enroll four synthetic identities and five devices through the production CLI."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import traceback
import uuid

from lib.package_environment import fixture_environment
from lib.transport_actors import enroll_actors
from lib.transport_fixture import FixtureError

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("instance", "admin-config", "binary", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--resume", action="store_true", help="Recover the saved original enrollment IDs; never creates a new plan")
    args = parser.parse_args()
    os.umask(0o077)
    logs = ROOT / ".local/gate6-transports/private/actor-commands" / uuid.uuid4().hex
    logs.mkdir(parents=True, mode=0o700)
    environment = fixture_environment()
    binary = args.binary.resolve()

    def execute(name, arguments, data=None):
        incoming = logs / (name + ".input")
        incoming.write_bytes(b"" if data is None else json.dumps(data).encode())
        output = logs / (name + ".output")
        with incoming.open("rb") as inp, output.open("xb") as out, (logs / (name + ".stderr")).open("xb") as err:
            child = subprocess.Popen([str(binary), *map(str, arguments)], cwd=ROOT, env=environment,
                stdin=inp, stdout=out, stderr=err, start_new_session=True)
            joined = False
            try:
                code = child.wait(timeout=30)
                joined = True
            finally:
                if not joined:
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    child.wait()
        if code != 0:
            raise FixtureError("fixture-actor-public-command-refused")
        return output.read_bytes()

    try:
        result = enroll_actors(ROOT, args.instance, args.admin_config, args.binary, args.output, execute, resume=args.resume)
        print(json.dumps(result, sort_keys=True))
        return 0
    except Exception as error:
        (logs / "failure.log").write_text(traceback.format_exc())
        print(json.dumps(dict(enrolled=False, code=str(error) if isinstance(error, FixtureError) else "fixture-actor-enrollment-failed",
            name=type(error).__name__, line=traceback.extract_tb(error.__traceback__)[-1].lineno,
            cell_qualified=False, task_complete=False)), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
