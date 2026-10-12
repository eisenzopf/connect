# Contributing to Connect

Connect is an experimental server and SDK. Start with [the architecture](docs/ARCHITECTURE.md),
[the wire contract](contracts/connect-control-v1.md), and [the developer guide](docs/DEVELOPING.md).
The supported qualification host is macOS arm64 with the pinned Rust toolchain,
Node 22+ and Python 3.11+. Other platforms are not qualified yet.

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
and `--package` verifies an installed candidate. That historical binary is not
in Git; do not call missing historical-binary checks a pass. See
[local release instructions](docs/LOCAL_RELEASE.md). Hosted CI has not run until
the branch is pushed; local results are recorded separately.

Keep credentials, provider payloads, private call data and installation state
under ignored `.local/`. Never attach raw private logs to issues. Upstream fixes
belong in digest-checked patches and their upstream PRs; avoid invisible `.deps`
edits. Do not test a migration against someone's working database.

Public review does not imply a production-ready release. Reviewed SMS,
Connect-carried media, agent tools and task continuation are qualified within
the recorded local topology. The full connector kit, video and broader platform
qualification remain planned. Read the [Gate 4 review](docs/evidence/gate-4/acceptance.html)
and its explicit limits before expanding a claim.
