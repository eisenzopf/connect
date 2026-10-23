# Native runtime media controls

This Gate 6 profile applies authenticated `connection.update` requests to an
already negotiated native Connection. Each request names its Conversation,
Session, Connection, logical operation and expected stream revision. The actual
physical admission, current credential and configured wire scope must match the
retained native controller. Capabilities and request assertions grant nothing.

The host composes the original invocation's real current authority with the
explicit `MediaControlIntent::Gate` hook. That hook receives the complete target
set, action and expected revision. It must derive those facts from the same
invocation and retain its actual grant through the transition. It must not
reacquire an owned reader behind a queued writer or cache the grant in idle ports.
Normal publication and forwarding continue to obtain fresh authority separately.

## Actions and directions

Directions describe the requesting endpoint, consistently with native offers.
The runtime gate intersects the originally negotiated directions; it cannot add
a capability or permission that was absent from negotiation.

- `hold`: selects all current logical streams unless `streams` explicitly names
  a nonempty subset. By default Core remains `sendonly`: the endpoint can
  receive authorized playback while its publication is suspended, consistently
  with the protocol's gateway hold direction. A `details` object containing only
  `{"direction":"sendonly"}` explicitly selects the endpoint's sending direction
  instead and suppresses its playback. Explicit endpoint `recvonly` has the
  default behavior. Intersection with an
  originally one-way binding can leave neither direction enabled.
- `resume`: clears the selected hold mask and restores the originally negotiated
  directions. It preserves an independently requested source mute. It cannot
  promote an originally one-way binding into bidirectional media.
- `mute`: requires a nonempty `streams` list and suppresses those streams'
  endpoint-to-Core publication. It preserves playback from other sources to the
  endpoint. A binding without a negotiated publication direction refuses mute.
- `unmute`: clears the selected source mute, subject to current authorization.
  It does not clear an independent hold or create a missing publication direction.

`streams` contains unique existing logical IDs, with at most 32 targets. Unknown
IDs, duplicate/empty target lists, stale revisions, unsupported detail shapes,
foreign physical peers and changed/denied authority refuse the operation before
changing its gates. `codec_preferences` is not silently ignored by gate actions.
Codec renegotiation remains a separate parameter negotiation; this gate entry
point explicitly refuses that action rather than pretending it changed codecs.

## Atomic transition and existing media

Core validates the complete target set before draining any port. It temporarily
stops admission to the affected delivery generations and waits for their actual
admitted IO. Cancellation or a deadline before commit restores the old admission;
no subset of the requested gate state is committed. Final validation and all gate
changes commit under one current-authority lease and one stream revision.

Mute/hold changes keep the negotiated codecs, logical IDs, transport IDs, native
handles, transport generations and cumulative opening budget. Their private
delivery generations advance so queued prechange media cannot reappear after
unmute or resume. Actual IO already holding a delivery guard drains before the
change commits. A source mute does not suppress newly authorized playback.

The model keeps hold and source mute separately and preserves those choices
across logical transport replacement. Runtime controls also retain the exact
existing native parameter confirmation across their new graph revision. They
must not leave the lifecycle observer waiting for an unnecessary new offer.
They do not mark a changed codec, handle or resource as confirmed.

## Results and uncertainty

`control_native_media` returns historical local commit metadata. An application
can acknowledge that local control result, but it must not fabricate readiness,
Participant presence or durable operation status from it. The qualification
handler explicitly labels its response `local_control_commit`. Its diagnostic
`send` and `receive` flags use the Core perspective (Core-to-endpoint and
endpoint-to-Core respectively).

Cancellation does not prove that no mutation happened. Preserve the original
logical operation ID and reconcile the journal/current retained state before
retrying an uncertain outcome. Production canonical UCTP, HTTP/CLI/MCP control
facades still need the proper shared service/journal integration; this component
does not create a parallel business service or log.

Connection end, public subscription operations, complete codec-update handling,
production initial media/Participant admission and automatic refresh are separate
unfinished Gate 6 work. The checks here use isolated local TLS fixtures with a
controlled relationship/Store policy. Full Gate 6 Linux x86_64 and remote/native/
browser lanes remain required. Named external/provider rehearsals require the
concrete bounded G6-093 configuration and designated endpoints; publication is
separate.
