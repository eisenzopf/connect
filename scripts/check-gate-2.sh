#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir/.."
bash scripts/prepare-dependencies.sh
bash scripts/format.sh --check
(cd sdk/typescript && npm ci && npm run build)
npm run format:check --prefix sdk/typescript
node --test sdk/tests/clients.test.mjs
PYTHONPATH=sdk/python python3 -m unittest discover -s sdk/tests -p 'test_*.py'
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/check-recovery.py
python3 scripts/check-crash-replay.py
python3 scripts/check-examples.py
python3 scripts/check-example-recovery.py
git diff --check
# This is offline/local qualification only. Provider account operations and live
# SMS need a separate, redacted evidence record; they are never inferred here.
