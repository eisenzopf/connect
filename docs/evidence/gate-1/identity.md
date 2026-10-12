# Local server and identity evidence

2026-10-07, macOS arm64, Rust 1.97.1. Dependency identities are in the source lock.

- `cargo test --locked -p connect-store --test identity`: 2 integration tests passed.
  Identity/installation persistence, alias conflicts, scoped discovery,
  restricted administration, revocation after reopen, and newer schema refusal.
- `cargo test --locked -p connect-server --test local_identity`: 2 tests passed.
  Real native QUIC, mandatory certificate and hostname verification, restart,
  preserved initialization credentials, current-connection revocation and
  fresh-handshake rejection. Config rejects public binds and invalid limits.
  State directory is 0700; credential/key/database files are 0600.
- `python3 scripts/check-identity.py`: exit 0. Independent CLI processes initialized,
  preserved repeated init, started server, checked health, resolved each scoped
  identity in both directions, restarted server and stopped via SIGTERM twice.
- `cargo clippy --locked -p connect-protocol -p connect-store -p connect-service
  -p connect-client -p connect-server --lib --bins -- -D warnings`: exit 0.

Health is a network operation, not an existence check for a PID/state file.
The server refuses unsupported schema or failed bind before printing ready.
The local operator CLI can revoke credentials; it does not expose administrative
mutation through app messaging. Store verifiers are hashes of random 256-bit
credentials, with per-credential current scope/expiry/revocation checks.

Additional checks: `cargo test --locked -p connect-server --test maintenance`
passed all 4 tests: expired certificate refusal, occupied-port startup failure,
lock release after failure, explicit stopped-server TLS rotation, rejection of
old trust, refreshed trust acceptance, IPv6 loopback, and operator-only minting
with revocation checked inside the issuance transaction. The process recovery
script also proves CLI issue/revoke, app issuance denial and single-instance
startup refusal. TLS has an explicit one-year lifetime; rotating never resets
identity or application state.

The native request interface now supports all eight application operations.
The convenience SDK facade, independent app/agent examples and installable
package remain later gates. These results do not qualify Gate 1 as a whole.
