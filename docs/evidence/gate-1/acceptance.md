# M5 — local Gate 1 acceptance

Candidate: `connect-0.1.0-alpha.1-macos-arm64-263fe1592ceb`; source `263fe1592ceb5099985e58566ae3e3e35a34bc07`.

`bash scripts/check-local.sh` exited 0: 13 phases passed, 37 workspace tests and
doctests, 941 pinned upstream tests, zero failed or ignored. It runs real native
processes, SQLite/crash recovery, wire parity/authority/pressure, SDK and app
checkpoint cases. Private logs are under `.local/checks/20261008T011957.696743Z/`.

`python3 scripts/check-package.py dist/connect-0.1.0-alpha.1-macos-arm64-263fe1592ceb` exited 0: all nine
qualification phases passed. Normal binaries came from the installed runtime
archive; only abrupt-fault injection used explicitly featured source builds.
The SDK compiled from the extracted full source archive after fresh dependency
preparation. Private logs are under `.local/package-checks/20261008T012226.296671Z/`.

The full suite ran source `02b568524ae8cd51827095788654b9f5666bb9ce`. The only source-input
change afterward was the package checker clearing the shell's `RUST_LOG=warn`
override and normalizing terminal colors to test default logging. The installed
runtime and all Rust/tests/recovery scripts are unchanged; package qualification
ran the corrected harness. `result.json` records both revisions and all hashes.

- A1: Fresh local install, safe initialization and persistence — passed. Evidence: installed-walkthrough, restart-shutdown, identity-process, local_identity, maintenance.
- A2: Both app and agent initiate conversations, receive and reply — passed. Evidence: check-examples.py, installed-walkthrough, retained offline walkthrough.
- A3: Real UCTP/QUIC with explicit TLS trust — passed. Evidence: dispatch, local_identity, maintenance, installed-walkthrough.
- A4: Shared QUIC/WS semantics, fixtures and cross-transport replay — passed. Evidence: transport_parity: 4 passing cases, transport_pressure: 4 passing cases.
- A5: Uncertain acknowledgment, logical replay, changed-input conflict — passed. Evidence: check-crash-replay.py: before-commit/after-commit/before-ack, conversations, sdk_facade.
- A6: Offline catch-up, pagination and app-owned action deduplication — passed. Evidence: check-examples.py, check-example-recovery.py, native_messaging, transport_pressure, retained offline walkthrough.
- A7: Current authority, scope, spoofing/cursor denial and revocation — passed. Evidence: transport_parity, transport_pressure, local_identity, bounds, check-recovery.py.
- A8: Bounded queues/capacity; slow observer recovers journal pages — passed. Evidence: transport_pressure, bounds, conversations, recovery receipt capacity.
- A9: SQLite failure atomicity, consistent backup/restore, schema compatibility — passed. Evidence: conversations SQLite FULL, recovery, check-recovery.py, retained backup/restore.
- A10: Close, cancel/deadline, bounded shutdown and safe diagnostics — passed. Evidence: sdk_facade, transport_pressure shutdown regression, identity-process, installed-walkthrough logs, restart-shutdown.
- A11: Fresh documented package journey and usable native SDK source — passed. Evidence: sdk-source-build, check-examples.py, installed-walkthrough, retained walkthrough, connect-client compiling doctest.

Source-based WS parity/pressure and SDK authority/cancellation evidence is
explicitly separate from installed native CLI/process evidence. No production,
carrier, live Vapi/Telnyx, WAN or complete-v0.1 claim is made.
