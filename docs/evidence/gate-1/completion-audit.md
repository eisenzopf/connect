# Gate 1 completion audit

Scope derives from the numbered implementation contract and the delivery
plan's A1–A11 acceptance matrix. All 53 stable IDs are retained. This audit
covers Gate 1, not the subsequent full v0.1/provider/media roadmap.

Authoritative evidence inspected:

- Current protocol/store/service/client/server/example sources, crate boundaries,
  migrations, source lock and reproducible patches.
- Complete local runner results and private logs: 13 passing phases,
  37 workspace tests/doctests, 941 upstream tests, no failed/ignored cases.
- Archive checksums, runtime manifest/file hashes, inventory and normal
  binary absence of test controls; candidate source
  `263fe1592ceb5099985e58566ae3e3e35a34bc07`.
- Extracted SDK source compiled after freshly preparing the pinned graph.
- Nine installed package qualification phases, independent normal processes,
  source fault drills with installed recovery, and a second retained fresh
  install/offline/backup/restore walkthrough.
- Current process inventory: no task-owned servers/workers remain. Retained
  release, state and protected backups exist; prior milestone material remains.
- HTML IDs, dependency graph, status/evidence rules, local links and script syntax.
  The existing page design is preserved; a new visual render is not claimed.

Numbered item audit follows. Output paths now name actual artifacts rather than
the plan's earlier speculative module filenames.

| Item | Deliverable | Completion evidence |
|---|---|---|
| G1-001 | Record the local build environment | environment.md; dependencies.md; baseline.md; dispatch.md; decisions 001/002; fixtures; final suite |
| G1-002 | Create the minimal Connect workspace | environment.md; dependencies.md; baseline.md; dispatch.md; decisions 001/002; fixtures; final suite |
| G1-003 | Pin and reproduce the reusable sources | environment.md; dependencies.md; baseline.md; dispatch.md; decisions 001/002; fixtures; final suite |
| G1-004 | Run the relevant upstream regressions | environment.md; dependencies.md; baseline.md; dispatch.md; decisions 001/002; fixtures; final suite |
| G1-005 | Map public operations onto the wire | environment.md; dependencies.md; baseline.md; dispatch.md; decisions 001/002; fixtures; final suite |
| G1-006 | Prove native application dispatch | environment.md; dependencies.md; baseline.md; dispatch.md; decisions 001/002; fixtures; final suite |
| G1-007 | Set limits and state ownership | environment.md; dependencies.md; baseline.md; dispatch.md; decisions 001/002; fixtures; final suite |
| G1-008 | Close the baseline gate | environment.md; dependencies.md; baseline.md; dispatch.md; decisions 001/002; fixtures; final suite |
| G1-009 | Define isolated local paths and config | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-010 | Implement safe initialization | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-011 | Configure verified local TLS | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-012 | Implement server lifecycle and health | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-013 | Persist identities and aliases | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-014 | Issue and revoke scoped credentials | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-015 | Implement authorized resolution | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-016 | Evaluate the TypeScript native path | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-017 | Run the early local datagram probe | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-018 | Close the server and identity gate | identity.md; datagram-probe.md; decision 003; local_identity/maintenance; identity-process |
| G1-019 | Create transactional application storage | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-020 | Implement idempotent create and close | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-021 | Commit messages and recipient events atomically | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-022 | Implement request-result recovery | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-023 | Implement authorized history and journal pages | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-024 | Implement wait/resume without race losses | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-025 | Enforce capacity and explicit failure states | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-026 | Inject crash and retry scenarios | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-027 | Implement consistent backup and restore | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-028 | Define migration and rollback compatibility | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-029 | Close the storage gate | storage.md; store/service suites; installed crash/recovery; upgrade.md |
| G1-030 | Integrate QUIC with the shared service | transport.md; transport_parity (4); transport_pressure (4); all operation fixtures on both wires |
| G1-031 | Add the WS parity entry point | transport.md; transport_parity (4); transport_pressure (4); all operation fixtures on both wires |
| G1-032 | Publish executable protocol fixtures | transport.md; transport_parity (4); transport_pressure (4); all operation fixtures on both wires |
| G1-033 | Test auth and revocation across transports | transport.md; transport_parity (4); transport_pressure (4); all operation fixtures on both wires |
| G1-034 | Test reconnect, loss and backpressure | transport.md; transport_parity (4); transport_pressure (4); all operation fixtures on both wires |
| G1-035 | Close the native and parity gate | transport.md; transport_parity (4); transport_pressure (4); all operation fixtures on both wires |
| G1-036 | Implement the public Rust SDK facade | sdk-examples.md; public SDK doctest; sdk_facade (2); installed examples and app crash recovery |
| G1-037 | Define SDK retry and cancellation behavior | sdk-examples.md; public SDK doctest; sdk_facade (2); installed examples and app crash recovery |
| G1-038 | Build a persistent review-app example | sdk-examples.md; public SDK doctest; sdk_facade (2); installed examples and app crash recovery |
| G1-039 | Build the deterministic agent example | sdk-examples.md; public SDK doctest; sdk_facade (2); installed examples and app crash recovery |
| G1-040 | Build the three-terminal local demonstration | sdk-examples.md; public SDK doctest; sdk_facade (2); installed examples and app crash recovery |
| G1-041 | Replace sketches with tested SDK docs | sdk-examples.md; public SDK doctest; sdk_facade (2); installed examples and app crash recovery |
| G1-042 | Close the SDK and example gate | sdk-examples.md; public SDK doctest; sdk_facade (2); installed examples and app crash recovery |
| G1-043 | Implement the local verification command set | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-044 | Build the local installable candidate | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-045 | Install into an isolated local release directory | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-046 | Initialize and start the packaged server | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-047 | Run packaged app and agent scenarios | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-048 | Run the complete local fault/authority matrix | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-049 | Prove consistent backup and restore locally | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-050 | Rehearse upgrade and rollback locally | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-051 | Review local logs and packaging hygiene | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-052 | Complete a fresh local walkthrough | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |
| G1-053 | Sign off the local Gate 1 result | acceptance.md; result.json; nine installed qualification phases; upgrade.md; hygiene.md; walkthrough.md |

Application invariants: current authority is checked inside the transaction,
including cached logical receipts; effects/receipts/recipient events commit
atomically; changed logical input conflicts; SQLite failures are not successful
acceptances; the journal is authoritative; wakeups are hints; cursors bind
installation/identity/resource and advance without skipping unread authorized
entries; finite limits and wire byte budgets are enforced. Source assertions
and real process faults/pressure tests exercise these cases.

The fresh package walkthrough proves runtime independence from the checkout;
SDK compilation is separately a source-build requirement. WS parity/pressure
and SDK cancellation are source conformance evidence, not live provider or
browser media qualification. The agent's own SQLite effect is deduplicated;
arbitrary external effects are not claimed exactly once.

A shutdown regression was reproduced before its fix, and all pressure tests
passed afterward. Package log verification initially failed because this host
sets `RUST_LOG=warn`; the corrected default-log harness then passed. The failures
are recorded in transport.md and hygiene.md rather than omitted from history.

The final report revision follows the compiled candidate solely for
documentation, checklist statuses and redacted evidence. Production Rust,
lockfile, source patches, verification scripts and qualified binaries remain
identical to the candidate. Local scope exclusions and follow-on v0.1 work remain
explicit. No remote deployment or publication was performed.
