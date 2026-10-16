# 014 — Gate 6 clean contract version

2026-10-15; G6-003 / G6-010. The owner explicitly permits breaking pre-alpha API,
SDK and schema changes and rejected legacy API compatibility work.

The target is UCTP envelope 2, native ALPN uctp/2 and application profile connect/2.
The existing eight-byte RTP datagram header remains version 1 because its byte
format does not change. HTTP/3 ALPN remains h3. The SDK/server package release
version is independent and is not chosen as v0.2 automatically.

Canonical communication verbs dispatch once through Connect's service/store.
Existing Connect request dialects and route-ID-oriented public SDK methods may
be removed. No old-client matrix, downgrade path or compatibility facade is
required. The server refuses unsupported envelope/profile/schema versions
explicitly before acting.

At the new schema boundary, inspect the installation version before opening
mutable state. If incompatible, stop with backup and explicit reinitialization
instructions. Do not silently delete/migrate the installation or widen credential
permissions. A Gate 6 backup must restore with its matching schema/runtime;
automatic v0.1 conversion is not required.

Retain product capabilities and adapt meaningful behavior tests to the new API.
Old serialized field-shape tests are not release obligations. Preserve historic
v0.1 release evidence with its original source and limits.

## Implemented state boundary

Gate 6 state uses SQLite schema **20**. Opening a different schema stops before
mutable configuration or cleanup. `migrate` does not convert historical state.
The current runtime restores only a matching Gate 6 backup. A restored connection
row preserves its ID/lineage but requires fresh adapter observations; it does not
restore a physical call.

Keep the existing installation and its matching binary. Stop that server and use
its supported backup command to preserve the database, keys, configuration and
artifact blobs together. Keep provider secret references and their protected
vault separately as described by the existing operator recovery instructions.
Do not edit `PRAGMA user_version` to bypass refusal.

Initialize Gate 6 in a **new directory**. The source-checkout command is:

```sh
bash scripts/prepare-dependencies.sh
cargo run --locked -p connect-server --bin connect -- init --directory .local/gate6
```

Choose an unused directory; initialization refuses an existing installation.
New installation and credential IDs are intentional. This does not import old
conversations or continue an old provider engagement. Keep the prior installation
and verified backup until its owner chooses their disposition.

For a matching Gate 6 backup, the public CLI is:

```sh
connect backup --config STATE/operator.toml --server-config STATE/server.toml --output BACKUP
connect restore --backup BACKUP --directory NEW_STATE
```

`STATE`, `BACKUP` and `NEW_STATE` are explicit local paths; both destination
directories must be new. The backup includes artifact blobs and installation
recovery material. Treat it as private. Provider credentials in an external vault
remain external; a copied reference is not a copied secret.

Verification: `cargo test --locked -p connect-store` covers current state and
backup invariants. The server's `first_run_schema_boundary` and
`artifact_recovery` tests exercise the public CLI and file recovery.
