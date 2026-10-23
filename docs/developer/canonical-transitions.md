# Canonical transition admission

The signed canonical dispatcher now routes `session.consult`, `session.handoff`
and `connection.replace` into the existing durable transition ledger. It does
not fall through to stock core handlers or create a parallel conversation,
Session or operation store. This is G6-037 dispatch and admission work. Actual
consultation now uses the installed G6-039 coordinator described in the
[native consultation guide](native-consultation.md). Public handoff and device
switching use their own managed coordinators. Native and configured telephone
replacement now have actual private readiness and compound movement; see
[telephone endpoint replacement](telephone-replacement.md) for the configured
path and remaining G6-041 qualification. Admission alone cannot claim readiness.

Each command requires a saved `operation_id`, `cid`, `sid` and
`expected_revision`. Optional `connid` is the source connection assertion; it is
required for replacement. The payload retains the frozen typed fields from
[the canonical mapping](../../contracts/connect-uctp.md). In particular,
`retained_connid` does not replace `connid`, and a replacement selects exactly
one candidate connection or endpoint. Reply assertions are refused. Private
Participant IDs are normalized before signature-bound policy admission and
Store submission, so another ordering does not change logical intent.

`CanonicalClient::transition` accepts
`connect_client::transitions::TransitionCommand`. It validates intent, constructs
the canonical payload and scopes, signs a fresh envelope, and consumes the
correlated `operation.updated` plus completion ACK. Its result preserves
`accepted`, pending, cancelled and unknown states. A transport ACK does not mean
that media is ready or that a transition completed.

Unlike the explicit HTTP `TransitionRequest`, canonical payloads do not carry an
application expiry. The server selects a bounded 24-hour operation expiry on
first admission and persists it. Replays recover that exact expiry from the
current owner’s receipt; a fresh timestamp/signature never renews it or changes
the digest. HTTP-first operations retain their original explicit expiry on
canonical replay. HTTP status (`Operation::DurableOperation` get through
`/v1/operations`) exposes the persisted expiry for an explicit HTTP replay of a
canonical-first request. Both forms retain current authorization and intent
conflict checks. Tombstones cannot become new work.

The peer authority proof binds the normalized command, logical ID, all resource
assertions and revision. It includes the operation owner’s Identity writer,
Conversation writer and the current source Identity readers derived from the
actor, Session owner, selected Participants and Connections/endpoints. Replays
retain the original receipt’s current source footprint. An unselected Conversation member does not become an operation source merely
because a receipt now exists; replay retains the same source footprint. An
unrelated Identity is not locked just because it is in the same tenant. Store submission reuses this
owned proof; it cannot reacquire behind a waiting writer or substitute another
operation, resource, action or credential. Credential/key expiry is rechecked
before committing. The response lease retains the proof through the existing
physical signed reply path.

Ordinary HTTP/Store transition admission uses the same source-reader plan. Its
current scope, membership epoch, endpoint, topology, reservation, original
receipt and worker-recovery checks remain authoritative. Canonical admission
creates accepted work and reservations only; it performs no provider call,
Connection activation, role transfer or media graph commit. Cancellation and
`operation.get` use the shared decision/recovery engine.

Local checks include proof binding, source-reader exclusion and release, expiry
preservation across a later replay, HTTP-first parity without an independent
`operation.get` grant, and the existing Store transition crash/recovery suite.
Signed runtime tests exercise all three commands, public HTTP status/replay and
cancellation, changed-intent refusal, owner receipt recovery after departure without execution,
current recipient-policy replay refusal and invalid or
ungranted commands with no ledger effect. Their active media rows are explicitly
trusted fixture adapter observations. They do not prove physical media readiness
or completed consult/handoff/device movement. Full session/connection/stream
installation, coordinator execution, remaining approved application profiles,
obsolete-path removal and broader Gate 6 acceptance remain open. All checks are
local macOS arm64; no remote/provider/publication lane is included.

## Exact optional payload fields

`TransitionCommand` and `TransitionRequest` retain accepted optional fields in
`payload_extensions`. This map is part of the original durable intent, including
nested objects, array order and explicit nulls. Changing or omitting an extension
under the same logical operation ID conflicts across signed and HTTP transports.
The map cannot override a typed field, even an absent optional field, and its
serialized representation is limited to 32 KiB. Extensions grant no authority.

Both canonical Rust clients also expose `request_value` and
`request_value_with_cancel` for a validated original JSON payload. Their reply,
scope and cancellation checks are the same as the typed request methods. The
transition wrapper uses this path so typed payload serialization cannot discard
accepted fields. Fresh schema 47 refuses previous candidate databases whose
transition digests may already have lost optional fields.


G6-040 now has a controlled Store source boundary documented in the
[native handoff implementation guide](native-handoff.md). It pins accepted intent
and current authorization without claiming consent, native readiness or execution.
The ordinary signed/HTTP handoff path remains admission-only until the native
coordinator and its role/media custody are installed.
