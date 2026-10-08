# Run the qualified Gate 1 local candidate

These instructions and schema 1→2 drill describe the archived Gate 1 candidate.
They do not qualify the current Gate 2/Gate 3 development workspace. See
[index.html](index.html) for current plans and [SDK.md](SDK.md) for development interfaces.

This is Gate 1: the local server, native Rust SDK, and a deterministic review app
and agent. Vapi, Telnyx, SIP/PSTN/SMS, WebRTC, files, voice/video, MCP, and an
external assistant integration remain subsequent v0.1 gates. This candidate is
for macOS arm64. It is not published and requires no provider account.

## Install without replacing system software

Verify all three supplied archives in their download directory:

```sh
shasum -a 256 -c SHA256SUMS
```

Create a new local workspace. Extract the **runtime** archive into its `releases/`
directory; retain the versioned directory name. Do not unpack the source or
qualification archives into the runtime directory. Use the absolute path of
`releases/<candidate-id>/bin/connect` in place of `CONNECT` below, and its sibling
`connect-example` in place of `EXAMPLE`. These labels are placeholders, not shell
variables. No system PATH change, daemon, tunnel, DNS, or remote server is needed.

Run `CONNECT --version` and compare the version and binary hashes with the
included `manifest.json`. Keep state outside `releases/`.

## Start, communicate, restart

From your new workspace, in terminal 1:

```sh
CONNECT init --directory state --example review-loop --bind 127.0.0.1:4433
CONNECT serve --config state/server.toml
```

Choose a different loopback port with `--bind` if needed **before initializing**.
Initialization preserves a completed existing installation; it does not reset
credentials or state. An incomplete destination is refused. Configurations,
credentials, the local certificate/private key, and SQLite state are protected
local files. Never paste credentials into commands, logs, or issue reports.

In terminal 2:

```sh
CONNECT health --config state/operator.toml
CONNECT diagnostics --config state/operator.toml
EXAMPLE reviews --config state/reviews.toml
```

In terminal 3:

```sh
EXAMPLE agent --config state/agent.toml
```

Both processes initiate and reply using only the native SDK. The responder is
deterministic. Each owns its own `*.work.sqlite3` file for jobs, pending replies,
original logical request IDs, and inbox cursors. It does not read Connect's DB.

Stop both workers with Ctrl-C. With the server still running:

```sh
CONNECT demo verify --config state/operator.toml
```

The command runs the two sibling example binaries for a bounded interval and
checks their native conversations. Keep both binaries together. Repeat it after
restarting the server to check continuation. Restart an exited worker using the
same config/work file; automatic reconnection is not claimed. Cancellation does
not undo accepted work. Reuse the same logical request ID after an uncertain
send, or query `request_status`; never silently generate a replacement ID.

For offline recovery, stop the agent, send a review request through the SDK or
`CONNECT request --config state/reviews.toml` with a JSON operation on stdin,
then restart the agent. The source bundle's `scripts/check-examples.py` provides
a reproducible offline/replay scenario using the installed binaries.

## Back up, restore, upgrade

With the original server running or stopped:

```sh
CONNECT backup --config state/operator.toml --server-config state/server.toml --output backups/pre-upgrade
CONNECT restore --backup backups/pre-upgrade --directory restored --bind 127.0.0.1:4434
CONNECT serve --config restored/server.toml
```

Restore requires a new directory and verifies checksums/database integrity before
creating it. Use another port for the restored installation. It preserves the
installation identity, journal, receipts and cursors at the backup snapshot.
A cursor newer than that snapshot is rejected. App-owned work files and
separately issued credential files are separate recovery material; back them up
with their owners. Backups contain credentials and keys, not just public data.

`serve` never implicitly migrates. Stop the affected server, retain a verified
matching backup, then run:

```sh
CONNECT migrate --config state/operator.toml --server-config state/server.toml --backup backups/pre-upgrade
```

The current development migration supports schema 1–5 to 6. The historical
Gate 1 archive qualified schema 1 to 2. Unsupported newer schemas and stale or
foreign backups are refused. Roll back by restoring the matching **pre-upgrade**
backup into a new directory and using the previous compatible binary. Do not
downgrade a DB in place. Work accepted after the backup is absent from rollback.

Stop foreground workers and server with Ctrl-C. Keep the installation and
protected backups for review; do not delete unrelated directories or processes.

## SDK source and developer qualification

The `-source.tar.gz` archive contains the complete workspace at the candidate's
source revision, including Rust/Python/TypeScript SDKs, lockfile, patches, instructions,
and numbered HTML plan. Runtime binaries require neither this archive nor Rust.
SDK builds require the pinned Rust toolchain, native build tools, and dependency
preparation; see `docs/DEVELOPING.md` and `docs/SDK.md` in the source archive.

The runtime archive also includes `sdk/python/connect_sdk`, the built TypeScript
client under `sdk/typescript/dist`, and `examples/agent-presence`. Native language
clients use the installed `bin/connect` helper; HTTP/JSON is a separate local
interface. Set `PYTHONPATH` to the extracted `sdk/python` directory or import the
TypeScript module from its extracted path. Python 3.11+ and Node 22+ are required;
no Rust compiler is needed to use these packaged clients. Package qualification
exercises both native QUIC and HTTP clients from these extracted runtime files.

The separate `-qualification.tar.gz` contains the actual archived schema-1
milestone binary and its checksum/provenance manifest. It contains no old DB or
credentials. That historical binary is not a public release or a claimed
reproducible source build. The drill initializes fresh schema-1 state with it.

From the source checkout, extract qualification material separately and run:

```sh
bash scripts/check-local.sh --previous-binary /absolute/path/qualification/connect-schema1
python3 scripts/check-package.py /absolute/path/to/candidate-directory
```

The candidate directory contains `SHA256SUMS` and the three archives. Raw logs
and machine-readable results stay in ignored `.local/checks/` and
`.local/package-checks/`. Qualification uses disposable loopback installations,
synthetic requests, bounded waits, and task-owned process cleanup. Test-only
fault binaries are built from corresponding source for crash drills; installed
normal binaries replay the results. No test fault controls ship in the runtime.
