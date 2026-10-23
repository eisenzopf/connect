# Native Connection and Stream lifecycle observations

This Gate 6 profile adds signed `connection.ready`, `stream.opened` and
`stream.closed` delivery to the native negotiation profile in
[native-media-profile.md](native-media-profile.md). These are observations of
native resources and IO. They do not admit a Participant, change public Session
presence or constitute a durable mutation receipt.

The pinned UCTP protocol, `docs/CONVERSATION_PROTOCOL.md` in prepared rvoip,
section 7.4, describes readiness as media flowing in the negotiated directions
and requires Stream opening after Connection readiness and observed frames.
Section 10.1 binds datagram handles to their physical peer. This implementation
uses exact per-binding observations; endpoint parameter confirmation alone is
insufficient for these events.

## Preparation and first traffic

The signed `native-prepared/1` setup discloses provisional handles before public
readiness. After validating that setup and answering the scoped server callback,
the endpoint can start the negotiated media on those handles. It must not wait
for `connection.ready` before starting that media. The server discards precommit
traffic; committed publication still requires current relationship, credential,
direction, codec and routing authority. No preparation response grants access.

For the first `connection.ready`, every current selected binding must have
observed the directions it negotiated. Endpoint-to-server RTP counts only after
the checked native ingress has successfully published to the exact active Core
port. Server-to-endpoint RTP counts only after guarded delivery successfully
submits actual native datagrams. A one-way stream needs its negotiated direction;
it does not manufacture an opposite-direction packet.

Reliable data has separate observation points: accepted contextual native data
publication and a successful local reliable-writer receipt. Submission attempts,
unknown outcomes, protocol acknowledgments and journal notifications do not count
as successful data writes. Actual application data remains governed by the
installed journal/outbox policy; this profile introduces no probe message or
competing business log. A local write never proves remote application delivery.

No fake media, forced self-subscription or peer ready boolean is used. The public
RTP lifecycle checks exercise actual communication between separate native peers.
Mixed-family parameter preparation is independently checked. Public reliable-data
lifecycle qualification must exercise its real application and journal policy;
RTP results do not prove it.

## Event ordering and replacement

`connection.ready` is emitted once for the Connection, after the native graph's
signed parameter confirmation and required observed IO. Its stream bindings name
the exact current codecs and physical-peer handles. `stream.opened` follows only
after that Ready frame has actually completed on the existing signed writer.
Neither queuing an event nor a peer assertion advances the announcement state.

For codec replacement, an announced transport produces one adjacent
`stream.closed` / `stream.opened` publication only after the confirmed replacement
has its own required flow. The closed observation preserves the earlier time at
which the old binding was observed retired; both envelope timestamps identify
the common publication. No half-pair is announced while waiting for new media.
The replacement event is authorized as one unit covering both bindings, and one
bounded physical-writer claim owns both frames. Current revision/flow and authority
are checked before every IO poll. Interruption after either frame may have begun
is uncertain and retires the sink before releasing its source reader.
Unchanged logical bindings retain their observations. Observations from a retired
transport, an old local handle, a different medium, another Connection or a prior
Connection generation cannot establish the replacement. An incompatible offer
continues to preserve the previous committed graph.

A transport EOF may prevent final hints from being written. Explicit Connection
end, general pause/resume and subscription handlers must compose with their own
checked lifecycle transitions; the observer does not invent those operations.

## Authority and writer custody

`native_lifecycle_registration` seals the exact successful source offer, retained
Core controller, member epoch, identity, endpoint/device, physical credential
incarnation and configured wire scope. A foreign offer cannot relabel an existing
Connection's lifecycle facts. The registration stores metadata and weak references
to initial authority; it does not keep the source invocation's grant reader.

Every event obtains a new `NativeLifecyclePolicy` authority lease. Its host policy
must validate the current principal, relationship, endpoint, exact resource and
event permissions while retaining the real grant through writer IO. Capabilities,
roles, old observations and endpoint assertions are not permission defaults.
Registration begins observing only after the source grant has left the writer.

Each event uses a bounded one-use permit on the existing physical writer. Queued
copies own the cancellable slot, not independent grant readers. The actual writer
claims the real guard at dequeue and rechecks the exact revision, binding and
physical/credential lifetime before each IO poll. Cancellation before IO preserves
the writer. Partial-write cancellation destroys its owned sink before releasing
that IO's authority and reports submission uncertainty. A pending event task is
owned by the announcement state and bounded by the event/authority deadline;
registration drop cancels its pending permit directly.

The physical lifecycle mux bounds registrations and reserves capacity for control.
A refused or expired registration is removed locally; it does not retire other
Connections sharing that physical peer. Reauthorization installs a new current
registration. Writer completion takes precedence over concurrent cancellation of
a spent permit, so a completed Ready advances ordering instead of being repeated.

## Qualification boundaries

The component's default tests use isolated local TLS fixtures and controlled
relationship/Store policy. They do not prove production Session admission,
presence, durable Session projection, automatic credential refresh, general
subscriptions, SIP/WebRTC/browser media or reliable-data business workflows.
Gate 6 still includes its approved Linux x86_64 and remote/native/browser lanes;
those need their own evidence. Named remote/provider rehearsals require the
concrete bounded G6-093 configuration and designated test endpoints. Publication
remains separate.

Cancellation is not proof that no local mutation happened. Preserve the original
logical operation ID and reconcile its journal/Core state before retrying an
uncertain negotiation. Native event hints cannot replace that authoritative state.
