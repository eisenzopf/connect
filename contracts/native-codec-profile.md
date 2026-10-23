# Native codec update profile

An authenticated `connection.update` with `action: codec-renegotiate` requests
new RTP parameters on its exact retained native Connection. Conversation,
Session, Connection, operation ID and expected stream revision are required.
The physical peer, credential, configured wire scope and current parameter
confirmation must match the retained controller. A second physical peer cannot
borrow the same owner's Connection.

The broker retains the last successfully committed peer capability catalog and
substrate as metadata. An unconfirmed or failed offer cannot replace that catalog.
Later offers must still match the route’s physical substrate before they can
replace retained metadata. It grants no authority. Every transition still requires the original invocation's
explicit held current policy, and publication/forwarding require fresh current
media and audience grants. No synthetic offer or fabricated authorization proof
is used to reinterpret an update.

`codec_preferences` is a nonempty ordered list of at most 32 codec names. Names
are bounded and normalized; duplicate normalized names are invalid. `streams`,
when present, names a nonempty set of at most 32 unique existing logical streams.
When omitted, it selects all current RTP streams. Reliable data retains its
parameters; explicitly treating a data protocol as an RTP codec is unsupported.
`details` is not accepted for this action.

Every selected stream must have a compatible preference in the current server
and last confirmed peer direction-specific catalogs. The complete target set
is validated before preparation. An incompatible target refuses the complete
update. Other streams retain their selected parameters. Logical IDs, direction,
hold and source mute survive compatible transport replacement.

Core prepares the complete retained program and the server sends one scoped
`connection.offer` during the ongoing update. Its `native-prepared/1` setup names
the original update ID, next revision and exact logical/transport bindings. The
peer must return an authenticated, signed `connection.answer` with matching
scopes, operation, substrate, streams and setup. Parameter confirmation is not
readiness, flow or Participant presence. There is no initial `connection.answer`
to the update itself; after local commit, the fixture acknowledges an explicitly
labelled `local_codec_commit` result.

Until the valid confirmation and atomic commit, the old media program remains
available. Invalid answers, timeouts, cancellation or changed authority cannot
commit a partial program. Prepared native handles are burned even on failure;
retry cannot reuse an uncertain old handle. Following a successful replacement,
queued old media and old handles cannot satisfy new binding readiness. The new
binding needs actual permitted traffic before lifecycle events.

Ordinary packet forwarding preserves encoding and refuses a codec mismatch.
Managed audio scenes own transcoding; matching direct-relay codec updates do not
prove a managed-scene codec transition. `connection.ready` is a one-time event;
a replaced logical binding emits `stream.opened` only after its new actual flow.
New commands on the physical actor queue while an invocation owns the signed
parameter reply lane; their expected revision is rechecked when they run.

These APIs return historical local graph metadata, not durable operation receipts.
Production handlers must preserve the logical mutation ID and reconcile their
existing journal/current state after an uncertain result. Production canonical
Session/journal/Store installation, public subscriptions and end handling, initial
Participant/media admission, complete inactive semantics, automatic refresh and
full Gate 6 application lanes remain unfinished. Local TLS fixtures do not close
Linux x86_64, remote/native/browser or named provider qualification. Named external
rehearsals require bounded G6-093 configuration; publication is separate.
