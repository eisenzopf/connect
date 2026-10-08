# Connect language SDKs

- Rust: `connect-client`, directly over verified UCTP/QUIC.
- Python: `sdk/python/connect_sdk`, Python 3.11+ on the qualified macOS host.
- TypeScript: `sdk/typescript`, Node 22+, build with `npm ci && npm run build`.
- HTTP/JSON: opt-in loopback `POST /v1/operations` on the Connect server.
- CLI: `connect request` for one operation; `connect stdio` for JSON-lines sessions.

Python and TypeScript `NativeClient` reuse the Rust SDK through a persistent
subprocess; they require a built `connect` executable and a protected client TOML.
Their `HttpClient` uses local HTTP/JSON instead. Transport modes are explicit.
Packages are source distributions for local review, not published npm/PyPI releases.

Every mutation needs a caller-owned `request_id`. Persist it and the exact inputs
before sending; reusing it with different inputs returns `conflict`. Provider
acceptance/delivery is separate from local acceptance. Inbox consumers save their
effects, processed event IDs and cursor atomically. The connector worker never
repeats an uncertain external mutation automatically.

See [the runnable presence example](../examples/agent-presence/README.md) and
`bash scripts/check-gate-2.sh` for local conformance tests.


The SDKs separate operations, protocol validation, errors, and transports.
TypeScript exports named result interfaces and a discriminated `Outcome` union.
Both consume the shared wire fixtures and reject malformed/oversized results.
Native-client failures are terminal; close the client and recover using the same
logical mutation ID. An application rejection leaves the client usable.

Run `npm test --prefix sdk/typescript` and
`PYTHONPATH=sdk/python python3 -m unittest discover -s sdk/tests -p 'test_*.py'`
for bounded helper/HTTP failure tests with no provider account. The complete local
runner includes these tests plus real Rust native-helper/HTTP parity and recovery.
