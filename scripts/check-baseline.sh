#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir/.."
bash scripts/prepare-dependencies.sh
export CARGO_TARGET_DIR="$PWD/target/upstream"
cargo test --locked --manifest-path .deps/rvoip/Cargo.toml -p rvoip-quic --test loopback --test e2e_full_stack
cargo test --locked --manifest-path .deps/rvoip/Cargo.toml -p rvoip-core --test conversation_lifecycle --test messaging --test message_store_pagination
cargo test --locked --manifest-path .deps/rvoip/Cargo.toml -p rvoip-uctp --test coordinator --test datagram_conformance --test pending_dispatch
cargo test --locked --manifest-path .deps/rvoip/Cargo.toml -p rvoip-sip --lib
cargo test --locked --manifest-path .deps/rvoip/Cargo.toml -p rvoip-webrtc --test data_message_adapter --test credential_diagnostics
# Recreate Parley's sibling layout in an isolated directory for its exact pinned source.
python3 scripts/check-parley.py
