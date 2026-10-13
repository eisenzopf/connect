# Developing Connect

## Plans before implementation

Start with [the docs index](index.html), [v0.1 scope](connect-v0.1-contract.html),
and the affected gate’s product/delivery plan.
[Gate 2’s plan](connect-gate-2-plan.html) defines chat and provider telephone acceptance;
[its checklist](connect-gate-2-implementation.html) records work and evidence.
Provider telephone call control is Gate 2. Native audio and browser WebRTC are
Gate 3, even where older G2 item IDs are retained. Gate 3 has a [product/delivery plan](connect-gate-3-plan.html),
[numbered implementation plan](connect-gate-3-implementation.html) and
[acceptance map](gate-3-acceptance.json). Its durable application model,
two-peer native UCTP/QUIC audio core, loopback Vapi attachment, local SIP,
authenticated WebRTC, cross-transport pairing and managed artifacts are
implemented and qualified through offline, installed-package and separately
invoked live checks. See the [Gate 3 review report](evidence/gate-3/acceptance.html). Local rvoip patches remain
accepted when pinned and reproducible. Basic availability and a small expiring preference override are qualified in Gate 3;
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

The qualified development host is macOS arm64. Install Rust through rustup;
`rust-toolchain.toml` selects 1.97.1. Node 22+, Python 3.11+, Apple command-line
developer tools and Homebrew are required. The Opus binding discovers the native
static library through pkg-config; without it the bundled fallback can fail with
CMake 4. Run:

```sh
brew install opus pkgconf
bash scripts/prepare-dependencies.sh
cargo check --locked --workspace
bash scripts/format.sh --check
python3 scripts/check-dispatch.py
bash scripts/check-baseline.sh
```

`bash scripts/check-local.sh` runs the general bounded local suite, including
format/lint, workspace tests and doctests, pinned upstream regressions, native
dispatch and media/TypeScript experiments, and independent process recovery.
Logs and a JSON result stay under `.local/checks/`. A failed check exits nonzero
and reports later steps as not run. The historical schema-1 binary and adjacent
manifest are required for the upgrade drill; use `--previous-binary PATH` when
they are outside `.local/milestones/m1/`. Fresh legacy state is generated rather
than copied from someone else's installation.

For account-free source checks from a full-history Git checkout, use
`python3 scripts/check-local.py --ci` or Gate 4's explicit
`python3 scripts/check-gate-4.py --source-only`. The CI workspace path retains
workspace-wide features, every unit/integration test and doctest, and the default
example compilation. It runs named targets in small groups and removes only
their completed generated executables; shared libraries and ordinary application
binaries remain. Test failures retain their outputs. Debug symbols/incremental
caching are disabled, application deadlines are unchanged, and disk headroom is
measured before each group. Historical baseline build caches may be retired only
after their executable and provenance manifest have been verified.

Full Gate 4 qualification additionally needs an explicitly selected installed
Codex CLI. The recorded alpha host is `codex-cli 0.162.0-alpha.17.2`; other versions
are not qualified by that result. Obtain Codex through the
[official CLI installation instructions](https://learn.chatgpt.com/docs/codex/cli),
then inspect `codex --version` and `codex exec --help` before selecting its absolute
path. The required interface includes JSONL, `--ignore-user-config`, inline MCP
settings and exact `exec resume` support. The generic installer does not promise
the recorded alpha version. Run the account-free host checks before treating a
different version as compatible:

```sh
python3 scripts/check-gate-4.py --bounded-workspace --codex-binary /absolute/path/to/codex
```

These fixtures use scripted loopback Responses and no model account. Actual
model behavior is separately invoked and requires the owner's selected model
authentication. SDK/MCP communication itself does not require Codex. See
[the host guide](CODEX_TASK_HOST.md) and [agent setup](AGENT_ACCESS.md).
The explicit `--bounded-workspace` storage mode retains every host phase and
records its selection in the result. Omit it to use Cargo's original aggregate
workspace command. The current `709cf82` candidate records the bounded mode
in its 42-phase qualification. The historical `dc73996` 41-phase runner predates
this option and must use its original commands; its result is not a qualification
of newer main.

After committing a clean candidate source, packaging requires a separately
retained schema-1 milestone executable and its adjacent provenance manifest.
They are not generated by dependency preparation or present in Git; a supplied
qualification archive carries them for an existing local candidate. Without that
verified historical input, source checks are available but full packaging is not
qualified. Use an explicit path:

```sh
bash scripts/package-local.sh --previous-binary /absolute/path/qualification/connect-schema1
```

This builds an unpublished release candidate under `dist/`. It includes separate
runtime with Python/TypeScript clients, complete SDK source, and historical qualification archives. Run
`python3 scripts/check-package.py /absolute/path/to/candidate-directory --codex-binary /absolute/path/to/codex` to
verify hashes, compile the extracted SDK, and exercise independent installed
binaries with offline recovery, abrupt fault replay, backup, upgrade/rollback,
private file modes, log redaction, installed native/HTTP language clients and shutdown. See
[the local release instructions](LOCAL_RELEASE.md). No remote deployment is
part of either command.

The invoking checker must match the candidate's recorded source. Use the checker
in that candidate's extracted source archive, or a full-history checkout at its
exact source revision. Current main may have later workflow or source changes
and must not be substituted silently. A source archive can build the SDK and
qualify its matching installed package; complete source qualification additionally
needs Git history. See the two paths in [local release](LOCAL_RELEASE.md#reproduce-qualification).

To add package qualification to the general local suite, both selectors are
required:

```sh
python3 scripts/check-local.py --previous-binary PATH --package CANDIDATE \
  --codex-binary /absolute/path/to/codex
```

`--ci` intentionally cannot qualify a package or the archived schema-1 migration.

`bash scripts/check-gate-3.sh` runs the focused Gate 3 offline contract, store,
authority, native media, loopback Vapi, SIP, WebRTC, cross-transport, artifact
and language-SDK checks. It never calls a live provider or changes an account. Its JSON and logs remain under
`.local/gate-3-checks/`; live Vapi, Telnyx SIP/PSTN and browser evidence is a
separate required qualification.

Use `scripts/format.sh` for formatting. `cargo fmt --all` recursively includes
upstream path dependencies and would invalidate their recorded patch hashes.
Do not edit `.deps/` to fix Connect behavior; record changes as explicit patches
and update the source lock after verifying them on a fresh prepared graph.

Each implementation checklist item closes with actual command/result evidence.
An existing test file or a successful build alone does not qualify runtime behavior.

The original project-page exploration is preserved under
`docs/concepts/connect-baseline.html`; the repository root is for build,
licensing and contributor entry points.

Update a numbered item with `python3 scripts/update-plan.py --gate 3 --item G3-001
--status done --owner YOUR_NAME --evidence PATH_OR_COMMAND_RESULT`. The updater
validates status, dependencies and completion evidence, and keeps the embedded
HTML task data and static task card synchronized. Omit `--gate` for Gate 1.
Do not mark an unrun qualification as complete.

The Gate 2 offline runner is `bash scripts/check-gate-2.sh`. It must
not provision accounts, purchase numbers, send SMS or place calls. Keep live
provider evidence separate. Historical Gate 1 package qualification does not
qualify new schemas, SDK methods or provider media paths.
