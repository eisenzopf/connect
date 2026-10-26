# Offline connector contribution template

This standalone Cargo package demonstrates the public Connect connector contract.
It supplies a deterministic messaging/context adapter, retained mock-provider
journal, authenticated callback fixture and bounded mono PCM voice lifecycle.
It imports no Connect store/service/server internals and makes no provider calls.
It is contribution material, not a supported carrier or a video implementation.

Read [the contributor guide](../../docs/CONNECTORS.md) and
[the versioned profile](../../contracts/connectors/profile-v2.md). From the Connect
repository root:

```sh
bash scripts/prepare-dependencies.sh
cargo test --locked --manifest-path examples/connector-template/Cargo.toml --target-dir .local/gate5/connector-template-target
python3 scripts/check-connector-contract.py --output .local/gate5/connector-report.json
bash scripts/format.sh --check
```

`src/lib.rs` implements messages, context attribution, immutable mutation receipts
and a provider fixture file. `media/voice.rs` keeps voice state and PCM framing
separate from messages. `tests/broken_fixture.rs` demonstrates expected rejection
when callback authentication is broken. The binary's `--contract-report` argument
requires a fresh isolated fixture path; it prints only named checks and limits.

The fixed fixture HMAC key is test data. Replace its verifier and resolve declared
secret references in a real provider adapter. Preserve uncertainty and use current
host authorization rather than granting authority to external IDs or context.
