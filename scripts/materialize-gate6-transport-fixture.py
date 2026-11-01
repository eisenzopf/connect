#!/usr/bin/env python3
"""Restore a matching private seed and emit server/operator fixture configuration."""
import argparse
import json
from pathlib import Path
import sys
import traceback

from lib.package_environment import fixture_environment
from lib.transport_fixture import FixtureError, materialize, owned_path, private_file

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=Path, required=True)
    parser.add_argument("--parameters", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        parameters = json.loads(private_file(owned_path(ROOT, args.parameters)))
        summary = materialize(ROOT, args.seed, args.output, parameters, args.binary, fixture_environment())
        print(json.dumps(summary, sort_keys=True))
        return 0
    except Exception as error:
        # Exception text may contain private paths/configuration. Fixed codes and
        # source coordinates are enough to locate a retained preparation error.
        code = str(error) if isinstance(error, FixtureError) else "fixture-materialization-failed"
        print(json.dumps(dict(materialized=False, code=code, name=type(error).__name__,
            line=traceback.extract_tb(error.__traceback__)[-1].lineno, cell_qualified=False, task_complete=False)), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
