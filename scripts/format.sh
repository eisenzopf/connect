#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir/.."
# Explicit packages keep formatting out of verified upstream path dependencies.
cargo fmt -p connect-connector-api -p connect-protocol -p connect-store -p connect-service -p connect-client -p connect-server -p connect-connectors -p connect-review-loop "$@"
