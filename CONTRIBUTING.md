# Contributing to Connect

Connect is an experimental server and SDK. Start with [the architecture](docs/ARCHITECTURE.md),
[the wire contract](contracts/connect-control-v1.md), and [the developer guide](docs/DEVELOPING.md).
The supported qualification host is macOS arm64 with the pinned Rust toolchain,
Node 22+ and Python 3.11+. Other platforms are not qualified yet.

## Make a change

1. Run `bash scripts/prepare-dependencies.sh` to materialize the audited upstream pins and patches.
2. Keep durable effects in the store, authorization in the service, wire types in
   the protocol crate, and adapters behind the [connector interface](docs/CONNECTORS.md).
3. Add behavior tests for the failure/recovery cases affected by your change.
   Preserve logical mutation IDs, current authorization, and journal replay.
4. Run `bash scripts/format.sh`, `npm ci --prefix sdk/typescript`,
   `npm run format --prefix sdk/typescript`, and `npm run build --prefix sdk/typescript`.
   Commit generated TypeScript `dist` with its source.
5. Run `python3 scripts/check-local.py --ci` for reproducible source checks.
   They use disposable local state, no provider account, and no live telephone/SMS.
6. Open a PR describing the problem, resulting behavior, checks performed, and
   any wire-contract or migration impact. Update the relevant numbered gate items.

CI runs the source checks on macOS arm64. It does **not** replace release
qualification: the full local runner also exercises an archived schema-1 binary,
and `--package` verifies an installed candidate. That historical binary is not
in Git; do not call missing historical-binary checks a pass. See
[local release instructions](docs/LOCAL_RELEASE.md). Hosted CI has not run until
the branch is pushed; local results are recorded separately.

Keep credentials, provider payloads, private call data and installation state
under ignored `.local/`. Never attach raw private logs to issues. Upstream fixes
belong in digest-checked patches and their upstream PRs; avoid invisible `.deps`
edits. Do not test a migration against someone's working database.

Public review does not imply a production-ready release. Live SMS, the Connect
media bridge, dynamic presence and broader platform qualification remain explicit
limits; proposing them is welcome, claiming them as delivered is not.
