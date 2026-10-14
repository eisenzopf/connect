# Prism independent contributor fixture

A separate implementer wrote this fixture using only the frozen public connector
API and contribution guide for G5-023. Its retained synthetic provider ledger,
messaging/media implementation and intentionally broken modes remain independently
authored. It imports no Connect server, store, service, conversation or routing
crate. This standalone Cargo workspace depends only on the public connector API
and ordinary Rust libraries.

The original exercise exposed missing host projection schema guidance. During
integration, its illustrative host parser was replaced with the shared public
`host_context::HostContext` decoder. The bridge now preserves the exact documented
ordinary/task projection schemas, journal references and omissions; it does not
guess derived status from event-name prose. Provenance permits 1024 bytes within
the existing 8192-byte total context bound, allowing the documented task metadata
without treating it as a short identifier. These integration repairs are recorded
separately from the original independent authoring and its source digests. The unchanged voice implementation was mechanically
split into `src/media.rs` for the shipped example.

From the Connect checkout, keep all state, logs and build targets under `.local/`:

```sh
bash scripts/prepare-dependencies.sh
PRISM_TEST_STATE_DIR="$PWD/.local/gate5/prism-tests" cargo test --offline --locked --manifest-path examples/connector-contribution/Cargo.toml --target-dir .local/gate5/connector-contribution-target
python3 scripts/check-connector-contract.py --manifest-path examples/connector-contribution/Cargo.toml --output .local/gate5/prism-report.json
bash scripts/format.sh --check
```

The common harness requires fresh state for every run. It checks 40 mandatory
cases. Focused tests also cover input bounds, host-envelope replay, immediate
uncertain cancellation and exact audio-frame/terminal behavior. The original
negative controls invoke the actual adapter with deliberate defects:

```sh
cargo run --offline --locked --manifest-path examples/connector-contribution/Cargo.toml --target-dir .local/gate5/connector-contribution-target -- --contract-report .local/gate5/prism-broken-context.json --broken-mode context
cargo run --offline --locked --manifest-path examples/connector-contribution/Cargo.toml --target-dir .local/gate5/connector-contribution-target -- --contract-report .local/gate5/prism-broken-authentication.json --broken-mode authentication
cargo run --offline --locked --manifest-path examples/connector-contribution/Cargo.toml --target-dir .local/gate5/connector-contribution-target -- --contract-report .local/gate5/prism-broken-uncertainty.json --broken-mode uncertainty
```

Each negative run needs a fresh filename and must exit 1. Context mode loses
historical attribution; authentication mode bypasses raw-byte HMAC verification;
uncertainty mode creates a second effect for a retained uncertain message. None
fabricates harness pass/fail fields.

Read the [guide](../../docs/CONNECTORS.md),
[fixture profile](../../contracts/connectors/profile-v1.md) and
[exact host projection contract](../../contracts/connectors/host-context-v1.md).
The fixed HMAC key is synthetic fixture setup, never a production credential.
Video, live providers, network quality, real-time capacity and concurrent
multi-process ledger writes are outside this exercise. Fixture qualification does
not establish release readiness or fresh carrier evidence.
