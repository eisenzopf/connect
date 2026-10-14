# Independent native control conformance

The [Connect control wire profile](../contracts/connect-control-v1.md#exact-native-reliable-control-framing-gate-5-protocol-kit)
is the implementation input. [Shared wire vectors](../contracts/fixtures/wire/)
and the existing application fixtures are language-neutral test data. Protocol
`v=1`, raw QUIC ALPN `uctp/1`, Connect profile `connect-control/1`, task profile
`connect-task/1`, server package version, and media datagram version are independent
identifiers. Incompatible envelope/profile inputs reject explicitly; no transport
fallback is allowed.

The [independent Python client](../examples/independent-uctp-client/client.py)
implements its own JSON framing, authentication, correlation, bounded exchange,
result validation and seven reliable control operations: resolve, create, send,
history, inbox, request_status and health. `send` supplies replies within an
existing conversation; inbox supports a saved opaque cursor. A mutation's stable
logical request ID survives uncertain exchanges; the caller chooses whether to
retry the original mutation. It imports only Python's standard library and
`aioquic`. It never imports a Connect SDK, launches a helper, or loads Connect or
rvoip protocol code. Tests audit these imports and exercise shared vectors.

Python 3.11+ with `aioquic==1.6.1` is the selected independent runtime. The
[upstream asyncio API](https://aioquic.readthedocs.io/en/latest/asyncio.html)
provides raw QUIC client connections and bidirectional streams; the
[QUIC configuration API](https://aioquic.readthedocs.io/en/latest/quic.html)
provides explicit TLS trust/ALPN and qlog instrumentation. The
[upstream repository](https://github.com/aiortc/aioquic) identifies RFC9000 QUIC
implementation and BSD licensing. The [1.6.1 release](https://pypi.org/project/aioquic/1.6.1/)
was published October 13, 2026 with macOS arm64 wheels. These primary sources
support the maintenance/runtime decision; the actual pinned dependencies and
all PyPI artifact SHA256 hashes are in the client's `requirements.txt`.
`aioquic` is BSD-3-Clause; transitive package notices remain their upstream
licenses (see [runtime notices](../examples/independent-uctp-client/DEPENDENCIES.md)).
No upstream source/parser modification is required. Pinned Connect/rvoip revisions
and patch digests stay in `dependencies/source-lock.json`.

From the repository root:

```sh
bash scripts/prepare-dependencies.sh
python3 scripts/prepare-independent-uctp.py
cargo build --locked -p connect-server --bin connect
.local/independent-uctp/venv/bin/python -m unittest discover -s examples/independent-uctp-client
.local/independent-uctp/venv/bin/python scripts/check-independent-uctp.py
```

For installed checks, pass `--binary <installed-connect>` and
`--client-root <installed-root>/examples/independent-uctp-client`; the checker
loads and launches that exact client artifact and records its SHA256. It does not
substitute checkout client code for the installed artifact.

The checker initializes fresh loopback-only state under ignored `.local/`, starts
a native server, and exchanges messages with separate native Rust CLI processes.
It checks exact identity/conversation/message IDs, durable message observations,
cursor reconnect, independent incoming contact, one logical effect on replay,
conflicting replay, server restart, malformed/foreign cursors, current revocation
on saved receipts, and revoked reauthentication. Dedicated connections prove
malformed JSON/envelope and >1MiB framing rejection by stream termination, not
merely timeout. Negative profile/version/message-size/unknown-field/unauthorized
operations are explicit. TLS server-name mismatch fails before authentication.
Client-side qlog packet/datagram counts and negotiated ALPN provide actual QUIC
path evidence; raw traces and TLS secrets are not published. Server logs/state
and each result are retained privately, including failures. No provider credentials
are inherited for the automatic driver lanes.

This is **independent reliable control proof only**. The client does not implement
media datagrams, capture, VP8 encode/decode, rendering, audio, browser WebRTC or
provider adapters. Media negotiation/routes and codec/frame vectors belong to
[the Gate 5 video implementation](connect-gate-5-implementation.html#G5-007).
A control-only pass cannot close the video or installed-release acceptance criteria.

The [Gate 5 driver](../scripts/check-gate-5.py) lists executable named source,
independent-client, installed, scripted-host, actual-model, operating and isolated
native-keychain lanes.
Use `--list` to inspect commands and immutable child budgets, `--source-only` for
explicit partial CI proof, or repeat `--lane` for selected coverage. Actual inference
requires its own explicit flag and matching `gate5-local` installed runtime/report.
The installed checker requires an explicit candidate profile and preserves all
archive/file/source hashes; retained Gate 4 baseline coverage is labeled accurately. Unselected
or unavailable required lanes remain unresolved; `whole_gate_qualified` stays false.
The retained operating lane qualifies existing task/audio profiles and explicitly
leaves the Gate 5 decoded-video profile unresolved until separate execution evidence.
