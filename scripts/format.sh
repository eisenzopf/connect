#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir/.."
# Explicit packages keep formatting out of verified upstream path dependencies.
cargo fmt -p connect-connector-http -p connect-connector-api -p connect-protocol -p connect-store -p connect-service -p connect-client -p connect-server -p connect-connectors -p connect-native-video -p connect-review-loop -p connect-mcp "$@"

# Independently buildable connector contribution fixture.
cargo fmt --manifest-path examples/connector-template/Cargo.toml "$@"

cargo fmt --manifest-path examples/connector-contribution/Cargo.toml "$@"

# Declaration-only shared SDK contract fixtures live outside Cargo workspaces.
rustfmt --edition 2024 contracts/sdk-api/rust/contract.rs contracts/sdk-api/rust/outline.rs contracts/sdk-api/rust/negative.rs "$@"
