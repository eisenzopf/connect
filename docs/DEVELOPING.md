# Developing Connect

## Plans before implementation

Start with [the docs index](index.html), [v0.1 scope](connect-v0.1-contract.html),
and the affected gate’s product/delivery plan.
[Gate 2’s plan](connect-gate-2-plan.html) defines chat and provider telephone acceptance;
[its checklist](connect-gate-2-implementation.html) records work and evidence.
Provider telephone call control is Gate 2. Native audio and browser WebRTC are
Gate 3, even where older G2 item IDs are retained. Gate 3 has a [product/delivery plan](connect-gate-3-plan.html),
[numbered implementation plan](connect-gate-3-implementation.html) and
[acceptance map](gate-3-acceptance.json), drafted for review. Its implementation
and media qualification remain outstanding. Local rvoip patches remain
accepted when pinned and reproducible. Basic availability and a small optional preference override are planned in Gate 3;
broader presence/preferences remain decision B-001.

## Repository layout

- `crates/connect-protocol`: serializable contract, public outcomes, errors, limits.
- `crates/connect-store`: SQLite schema, transactions, credentials, receipts, journal.
- `crates/connect-service`: authorization and shared application-handler implementation.
- `crates/connect-client`: native Rust SDK with explicit trust and request recovery.
- `crates/connect-server`: composition root, local CLI, listeners and owned provider workers.
- `crates/connect-connectors`: Telnyx/Vapi adapters and explicit private provisioning checkpoints.
- `sdk/python` / `sdk/typescript`: native-helper and local HTTP client facades.
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

`bash scripts/check-local.sh` runs the complete bounded local suite, including
format/lint, workspace tests and doctests, pinned upstream regressions, native
dispatch and media/TypeScript experiments, and independent process recovery.
Logs and a JSON result stay under `.local/checks/`. A failed check exits nonzero
and reports later steps as not run. The historical schema-1 binary and adjacent
manifest are required for the upgrade drill; use `--previous-binary PATH` when
they are outside `.local/milestones/m1/`. Fresh legacy state is generated rather
than copied from someone else's installation.

After committing a clean candidate source, `bash scripts/package-local.sh`
builds an unpublished release candidate under `dist/`. It includes separate
runtime with Python/TypeScript clients, complete SDK source, and historical qualification archives. Run
`python3 scripts/check-package.py /absolute/path/to/candidate-directory` to
verify hashes, compile the extracted SDK, and exercise independent installed
binaries with offline recovery, abrupt fault replay, backup, upgrade/rollback,
private file modes, log redaction, installed native/HTTP language clients and shutdown. See
[the local release instructions](LOCAL_RELEASE.md). No remote deployment is
part of either command.

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
HTML task data authoritative. This updater currently targets Gate 1 only. For Gate 3, use its HTML editor and
save the updated document, or update the embedded task data and static task card
together. Do not mark an unrun qualification as complete.

The Gate 2 offline runner is `bash scripts/check-gate-2.sh`. It must
not provision accounts, purchase numbers, send SMS or place calls. Keep live
provider evidence separate. Historical Gate 1 package qualification does not
qualify new schemas, SDK methods or provider media paths.
