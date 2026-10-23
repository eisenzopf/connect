# Configuring native quality cadence

G6-046 adds `connection.update` with `action: quality` and
`quality_interval_ms` from 100 through 60000. The default is 5000. It needs the exact
signed physical native Connection, current `connection.update` permission and
Conversation, Session, Connection, logical operation ID and expected stream
revision. A connection-creation grant, another physical peer, execution-owner
permission or an HTTP request cannot substitute for that authority.

Rust callers use `CanonicalClient::configure_native_quality(scope, interval_ms)`.
The returned operation can still be preparing, failed or uncertain. A completed
result confirms the actual local configuration and carries `connection_id`,
`connection_generation`, the unchanged `stream_revision`, `quality_interval_ms`
and `resources_changed: false`. It does not confirm remote receipt or playback.
No stream handle, binding, codec, source selection or RTP timebase changes.

Save the complete original scope and interval across uncertainty. Reusing the
operation ID with changed intent conflicts. Completed replay returns the original
historical result and never resets a newer cadence. Its generation identifies the
original Connection; the result does not grant access or prove that a later
incarnation uses the same configuration.

The Store records intention before the bounded native call. Its private attempt
binds the exact interval to the current original source. The engine's synchronous
policy callback inspects those held facts without acquiring another policy reader
or entering engine locks. The engine changes only its local quality cadence and
invalidates the old pending sample generation. Only an actual returned result
can create a completion receipt. Storage recovery records that receipt without
calling the engine again or retaining the original source readers. A retired
Connection is never revived by a delayed completion or a saved interval.

Quality reports still need fresh current source authority through their actual
signed delivery. Their reported `interval_ms` measures elapsed sampling time;
scheduler delay can make it larger than the configured cadence. Unmeasured loss,
jitter, RTT and MOS remain absent. See [native lifecycle events](native-lifecycle-events.md)
and the native-only profile in the [server HTTP/OpenAPI reference](../../contracts/openapi-canonical-http.json).

The local macOS arm64 component is verified with actual EdDSA and ES256 peers:
cadence changes and immutable replay preserve newer configuration and usable
audio; current named permission is enforced; captured completion recovers a
journal failure without repeating configuration or retaining source readers.
See the [scoped evidence](../evidence/gate-6/auxiliary-native-quality.json).
Capability advertisements, assurance/step-up, DTMF, continuous provider audio
signaling, Linux, browser, named remote hosting, acceptance and the complete
G6-046 task remain open.
