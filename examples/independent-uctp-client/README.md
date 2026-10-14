# Independent UCTP/QUIC control client

This Python implementation reads the published [wire profile](../../contracts/connect-control-v1.md)
and shared vectors. It uses `aioquic`'s independent QUIC/TLS stack and its own
Connect framing/parser. It has no Connect SDK/helper dependency and no media stack.
Requires Python 3.11+; automatic qualification targets local macOS arm64 only.

```sh
python3 scripts/prepare-independent-uctp.py
.local/independent-uctp/venv/bin/python examples/independent-uctp-client/client.py --config .local/example/reviews.toml
```

After authenticated readiness the client accepts one bare operation per stdin JSON
line and emits `{ "Ok": { "kind": ..., "data": ... } }` or a stable error code.
The TOML config uses `endpoint`, `server_name`, `trust_certificate` (DER),
`credential`, `timeout_ms`. Resolve relative certificate paths against the config.
Credentials stay in protected local files and never appear in command arguments.
Commands are serialized on one verified reliable QUIC stream with a fixed
per-exchange deadline. Supported operations: resolve, create, send, history,
inbox, request_status and health. Unsupported operations reject before send.

Persist the original mutation request_id until uncertainty resolves. Reconnect
with the same backing identity/credential and resume the last durably handled
inbox cursor. The client does not invent automatic retries or host wake behavior.
See [conformance instructions](../../docs/PROTOCOL_CONFORMANCE.md) for actual
positive/negative/native-participant checks and scope limits. Dependencies install
with exact versions and hashes from `requirements.txt`; fixture/client code uses
the repository MIT license.
