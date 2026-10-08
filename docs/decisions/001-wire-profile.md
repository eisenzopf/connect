# 001 — Reuse UCTP with an explicit Connect application profile

Adopt the pinned Parley ApplicationHandler interface and coordinator dispatch
hook. Add optional handler plumbing to UctpQuicConfig and each QUIC peer before
its input pump starts. The coordinator intercepts exact opt-in profiles and
returns immediately, preventing duplicate stock-core execution.

Use `connect.request`/`connect.result` extension envelope types with profile
`connect-control/1`; keep existing envelope and media meanings unchanged.
Stable participant identifiers belong to Connect's database, while validated
principal subjects identify individual credentials for operation-time
revocation. No sibling Developer checkout is needed to build the server.

The full Parley conference patch is preserved for provenance and interoperability
regressions. Connect's additional patch only wires the shared hook to QUIC.
No Thelve extraction occurs; both copied upstream sources are MIT-licensed.
