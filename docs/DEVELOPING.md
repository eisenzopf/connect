# Developing Connect

## Repository layout

- `crates/connect-protocol`: serializable contract, public outcomes, errors, limits.
- `crates/connect-store`: SQLite schema, transactions, credentials, receipts, journal.
- `crates/connect-service`: authorization and shared application-handler implementation.
- `crates/connect-client`: native Rust SDK with explicit trust and request recovery.
- `crates/connect-server`: composition root, local CLI and listener lifecycle.
- `contracts`: wire specification and executable conformance fixtures.
- `config`: public example configuration only.
- `examples`: independent review application and deterministic agent.
- `scripts`: reproducible developer checks, dependency preparation and local packaging.
- `patches` / `dependencies`: auditable upstream changes and pinned provenance.
- `spikes`: documented architecture experiments; code shares the workspace lockfile.
- `docs/decisions`: short architecture decisions; `docs/evidence/gate-1`: redacted results.

Generated sources are in ignored `.deps/`; build products in `target/`; private
state, credentials and raw evidence in `.local/`; distributable candidates in
`dist/`. No required source relies on an undocumented sibling checkout.

## Build and check

The project selects Rust 1.97.1. Run:

```sh
bash scripts/prepare-dependencies.sh
cargo check --locked --workspace
bash scripts/format.sh --check
python3 scripts/check-dispatch.py
bash scripts/check-baseline.sh
```

Use `scripts/format.sh` for formatting. `cargo fmt --all` recursively includes
upstream path dependencies and would invalidate their recorded patch hashes.
Do not edit `.deps/` to fix Connect behavior; record changes as explicit patches
and update the source lock after verifying them on a fresh prepared graph.

Each implementation checklist item closes with actual command/result evidence.
An existing test file or a successful build alone does not qualify runtime behavior.

The original project-page exploration is preserved under
`docs/concepts/connect-baseline.html`; the repository root is for build,
licensing and contributor entry points.

Update a numbered item with `python3 scripts/update-plan.py --item G1-001
--status done --owner YOUR_NAME --evidence PATH_OR_COMMAND_RESULT`. The updater
validates status, dependencies and completion evidence, and keeps the embedded
HTML task data authoritative. Do not mark an unrun qualification as complete.
