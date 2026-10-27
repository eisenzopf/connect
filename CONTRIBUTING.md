# Contributing to Connect

Connect is an experimental server and SDK. Start with the
[developer documentation](docs/index.html), [current protocol mapping](contracts/connect-uctp.md),
[source API references](docs/reference/index.html) and [workspace guide](docs/DEVELOPING.md).
The [operating profile](docs/decisions/015-gate-6-operating-profile.md) records
current tool/platform targets separately from measured qualification. Current
source results retain their local macOS arm64 scopes; Linux/remote qualification
remains open. The Gate 4/5 CI and recovery procedures below retain their named
historical scopes and do not qualify the current Gate 6 candidate.

Install the native media build dependency on this host with
`brew install opus pkgconf`. The Opus Rust binding uses `pkg-config` to find
the static library. CI installs the same dependency explicitly; without it,
the binding's older bundled source is incompatible with CMake 4.

## Make a change

1. Run `bash scripts/prepare-dependencies.sh` to materialize the audited upstream pins and patches.
2. Keep durable effects in the store, authorization in the service, wire types in
   the protocol crate, and adapters behind the [connector interface](docs/CONNECTORS.md).
3. Add behavior tests for the failure/recovery cases affected by your change.
   Preserve logical mutation IDs, current authorization, and journal replay.
4. Run `bash scripts/format.sh`, `npm ci --prefix sdk/typescript`,
   `npm run format --prefix sdk/typescript`, and `npm run build --prefix sdk/typescript`.
   Commit generated TypeScript `dist` with its source.
5. Run `python3 scripts/check-local.py --ci` and `python3 scripts/check-gate-3.py` for reproducible source and offline media checks.
   They use disposable local state, no provider account, and no live telephone/SMS.
6. Open a PR describing the problem, resulting behavior, checks performed, and
   any wire-contract or migration impact. Update the relevant numbered gate items.

CI runs source checks, the full offline media lane (including pressure and allocated queue checks), and the public schema-6 upgrade/rollback drill on macOS arm64. Build its pinned baseline with `scripts/build-recovery-baseline.py`; see the release instructions for the complete commands. It does **not** replace release
qualification: the full local runner also exercises an archived schema-1 binary,
and `--package CANDIDATE --codex-binary /absolute/path/to/codex` verifies an
installed candidate from its matching source. The selected host is separate
from ordinary SDK/MCP use; follow the explicit prerequisites in the
[developer guide](docs/DEVELOPING.md). That historical binary is not
in Git; do not call missing historical-binary checks a pass. See
[local release instructions](docs/LOCAL_RELEASE.md). Hosted CI results and local
qualification are recorded separately; a local pass does not imply a hosted pass.

Both hosted lanes preserve account-free coverage while bounding build storage:
workspace-wide features, unit/integration tests and doctests remain selected,
but completed test executables are retired between small target groups. Compiler
caches and ordinary application binaries are retained. Only a historical baseline
builder's generated target may be removed after its executable/manifest seal is
verified. No global toolchain, security or existing application-state cleanup is
performed. The workflow measures storage and retains redacted failed-phase log
tails and machine-readable summaries, rather than printing an unrelated passed
lint log. Hosted source-only mode remains partial selected-host qualification.

Keep credentials, provider payloads, private call data and installation state
under ignored `.local/`. Never attach raw private logs to issues. Upstream fixes
belong in digest-checked patches and their upstream PRs; avoid invisible `.deps`
edits. Do not test a migration against someone's working database.

Public review does not imply a production-ready release. Reviewed SMS,
Connect-carried media, agent tools and task continuation are qualified within
the recorded local topology. The [connector kit](docs/CONNECTORS.md) has
[current local source qualification](docs/evidence/gate-6/connector-template-audit.json).
Installed candidates, broader platform and complete browser/media qualification
remain separate. Use the [current engineering evidence](docs/engineering/index.html#gate-6)
and its explicit limits before expanding a claim.
