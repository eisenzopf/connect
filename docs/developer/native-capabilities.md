# Native capability advertisements

An endpoint publishes a complete replacement descriptor with
`CanonicalClient::advertise_native_capabilities(scope, participant, descriptor, trigger)`.
Keep the original logical operation ID, scope, participant, descriptor and trigger
when reconciling uncertainty. The native profile requires the current original
physical Connection, its expected stream revision and named
`capability.advertise` permission. Connection creation permission alone is insufficient.

Descriptors retain unknown extension values within explicit limits: 16 KiB,
32 entries per codec or feature list, 32 streams, and eight levels of JSON.
An empty descriptor or empty lists can withdraw capabilities. An advertised
assurance label must agree with the actual verified credential; it cannot
upgrade authentication. An advertisement never expands the current Endpoint,
Session, policy or media permission ceilings.

Replacement and operation completion commit in one Store transaction. The
completed result identifies the original Connection generation, unchanged stream
revision, participant, exact descriptor, trigger and advertisement record.
Replaying a completed older operation returns its historical result without
replacing a newer descriptor. Delayed completion cannot revive retired media.
Existing audio, codec bindings, handles and RTP clocks are unaffected. Codec
renegotiation remains an independent authorized operation.

The receive component is locally verified with actual signed EdDSA and ES256
peers, unchanged usable audio, exact replay, permission refusal and journal
recovery. See the [scoped evidence](../evidence/gate-6/auxiliary-native-capability-receipt.json).
Store schema 82 registers the new command while preserving the original native
source-binding checks. Prior database versions are refused without changing
their files; the first-run boundary requires an explicit fresh path.

The latest authorized declaration governs subsequent explicit codec updates
and new stream choices, within current Endpoint, Session and local engine limits.
A newly declared codec can be selected by a separate negotiation with a real
signed parameter answer. Withdrawal prevents future selection; it leaves existing
confirmed bindings usable, including changes to their source selectors. A completed
older advertisement replay preserves the newer declaration. Prepared negotiation
requests are checked against the current declaration generation before starting.
The [negotiation evidence](../evidence/gate-6/auxiliary-native-capability-negotiation.json)
records the actual gained-PCMA and withdrawal case for both signature algorithms.

The server queues listener hints after durable completion. An unsolicited signed
`capability.advertise` names the source `cid`, `sid` and `connid`, even though it
arrives on the recipient's own physical stream. It has no `operation_id` or
`in_reply_to`. The public SDK preserves it in its bounded event inbox when a
correlated command is pending; consume events with `receive()`.

Pending hints coalesce to the latest declaration, and completed replay does not
republish. Queues hold metadata. Actual writes acquire fresh joint source and
recipient authority and enforce both the original disclosure audience and the
current private scene. A healthy in-flight frame finishes before its replacement.
An unfinished write retires its sink before releasing source and recipient
readers. A completed receipt still does not confirm that another peer saw it.

Listener qualification remains in progress. The [delivery evidence](../evidence/gate-6/auxiliary-native-capability-delivery.json)
records the actual private consultation and restored public delivery cases,
intermediate failures and outstanding partial-write/authority checks. Owned provider audio now has a [continuous signed reader](native-owned-signaling.md), with its named local scope recorded separately.
The [HTTP/OpenAPI reference](../../contracts/openapi-canonical-http.json) describes
the native-only profile; HTTP cannot act as another physical native peer.
