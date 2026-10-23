# Owned native audio signaling

`PublicNativeAudio`, `PrivateNativeAudio` and `PreparedHandoffNativeAudio` own the
physical peer that completed their signed native preparation. A dedicated reader
then continuously frames and verifies that peer's reliable signaling, using the
same contextual server key and signature replay verifier as authentication.
Audio datagram waits and event-inbox waits can be cancelled without abandoning a
partial reliable frame. Cancelling an uncertain selector command still retires
its physical peer; keep its original logical operation ID for reconciliation.

Use `receive_signaling()` to read retained verified envelopes. Passive
`connection.ready`, `connection.quality`, `stream.opened`, `stream.closed` and
`capability.advertise` facts coalesce by kind and exact resource. Replies, data,
DTMF and other signaling retain arrival order. The inbox is bounded to 128 frames
and 2 MiB; exceeding a bound retires the peer instead of silently dropping a
control or signal. A capability fact's Connection scope identifies its source,
which can differ from this audio peer's Connection.

An idle signaling stream can outlive an RPC timeout. Once the first frame byte
arrives, the configured frame deadline bounds the rest of its prefix, body and
signature verification. Credential and server-key expiry bound the entire reader.
Malformed or incomplete frames, invalid signatures and verified terminal
authentication facts close the same physical peer, even without a foreground
signaling call. Already received terminal authentication facts remain available through the bounded inbox after retirement; further reads then refuse immediately. Cached facts cannot restart transport IO. Owner destruction stops the reader. This phase preserves the
terminal treatment of unsolicited assurance changes. An explicit owned step-up
completion installs a bounded expectation for its exact response, scope and
previous/required assurance. Only complete foreground validation can renew the
reader deadline. That exception is removed on every exit; an observed but
unvalidated transition closes the physical peer.

Use `refresh_credential()` or `refresh_credential_with_cancel()` on an owned
public, private or handoff audio peer to supply an explicit successor bearer.
The existing signed peer and its single reader perform the exchange. Only the
validated correlated `auth.session` updates the reader deadline; it remains
bounded by the original pinned server-key expiry. A refused refresh retains the
previous credential deadline. Receiving an unsolicited session envelope cannot
extend it. Deadline updates do not cancel or replace a partially received frame,
and an expired or retired reader cannot be revived. Media and signal operations
continue to require the server's current resource authority independently.

The installed Connect server now rotates the exact physical peer's existing
native controller during verified `auth.refresh`. Both credentials must be current
and enrolled to the same original Device/key. It records the actual Core
credential revision before selecting a successor Store media origin, preserving
the original offer, Connection, stream revision and media handles. Pending original
native controls must settle first. Success installs new dispatch registrations;
queued old signals, data and observations retain their original authority.
Refreshing with the unchanged credential updates dispatch metadata without
incrementing the media credential epoch. Possible submission or publication
uncertainty retires the physical peer and never retries a Core effect from rows.

The [native refresh evidence](../evidence/gate-6/native-credential-refresh.json)
records actual DTMF send/receive, same-credential refresh and independently decoded
RTP after predecessor expiry for EdDSA and ES256. These cases qualify the same
Device/key successor path. The
[signal-only continuation](../evidence/gate-6/native-signal-authority.json) now
qualifies narrower send-only consent and receive-only refresh with independent
current signal readers; it cannot grant RTP. Task-scoped step-up, production
fault/recovery profiles, exact auxiliary clauses and full G6-046 remain open.

The [scoped evidence](../evidence/gate-6/auxiliary-native-owned-signaling.json)
records controlled real QUIC framing/cancellation/expiry and the actual signed
Connect 220-descriptor burst, decoded duplex audio and selector replay for EdDSA
and ES256. Ordered DTMF in the controlled reader fixture is not durable Connect
DTMF integration. The 42 affected native/provider cases and the blocking-cleanup correction now pass their named local checks. Assurance/step-up and Gate 6 acceptance remain open.


Owned public/private/handoff audio also exposes `step_up_required()`,
`complete_step_up()` and its cancellation variant. A suspended typed command
reports `step-up-required`; obtain explicit current authority and complete the
original challenge rather than send another mutation. The
[owned SDK evidence](../evidence/gate-6/auxiliary-owned-step-up.json) records 80
passing SDK tests, including successful same-reader renewal beyond predecessor
expiry and refusal/malformed/cancellation cases. The
[production native path](../evidence/gate-6/native-media-step-up.json) now validates
actual user consent, exact physical credential rotation and fresh original-
command authority before resuming the same held native DTMF. Authentication,
original mutation outcome and explicit replay remain separate. Signal-only
permissions now qualify under the linked continuation; task profiles and full
G6-046 remain open.
