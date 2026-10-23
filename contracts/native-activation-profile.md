# Initial native media activation

A new native route begins with a protected `Preparing` Connection binding and
an empty managed Session. Current accepted room membership, endpoint ownership,
media grants and limits authorize preparation. They do not assert that media
resources are ready or that the Participant has been admitted.

Core issues `PreparedMediaObservation` only after every selected resource has
actually completed adapter preparation and is locally ready. Its constructor is
private. The observation identifies the exact controller, revision, connection
generation and complete binding set. It contains no current grant, credential
reader or public presence fact.

The installed policy must explicitly implement `control_prepared` to compose
that observation with its original owned control grant and protocol-specific
confirmation. The default continues to refuse a preparing binding. For native
media, `NativeMediaActivation.prepare` additionally requires the actual ongoing
broker operation, exact signed source offer, expected revision, full prepared
binding set and successfully verified signed endpoint parameter answer. A
client-ready field, an unrelated offer or resource readiness alone is insufficient.

The resulting protected binding changes no permissions. The engine checks
current authority, cancellation, deadline and all resources again before the
atomic media commit. The observation becomes usable by fresh publication policy
only after that commit. A cached observation from a failed or cancelled candidate
cannot activate a Connection. Native handles remain burned on failure.

Fresh policy calls can merge the committed resource fact with newly derived
current grants through `NativeMediaActivation.current`. No original owned reader
is retained by the idle handle. Current membership, endpoint, permission, lifetime
and generation checks still apply. A protected binding has an internal incarnation
that survives clones; reconstructing a new preparing binding with reused public
IDs cannot borrow the observation. Public logical IDs are unchanged. Retirement
or owner loss invalidates the live resource observation.

## Participant admission and flow

After the media commit, the coordinator uses the existing checked
`admit_peer_session_connection_with_held` transition. It retains the same physical
peer, actual invocation, actor, room, endpoint and logical operation identity.
Its Participant revision is captured before preparation and rechecked at commit.
The confirmation generation comes from the committed native resource observation.
The real staged scene and Participant controller commit membership and Connection
ownership; no synthetic `session.participant.joined` or public presence boolean is
used to substitute for that transition.

This admission is distinct from `connection.ready` and `stream.opened`. Native
lifecycle observations still require actual accepted traffic on the confirmed
binding. A precommit packet cannot establish readiness. Reoffers retain the
existing initial activation fact and do not repeat Participant admission.

The behavior checks use isolated signed QUIC/WebTransport peers with a controlled
current domain policy. They exercise an initially preparing route, an actually
empty managed Session, actual Participant admission, failure/retry and a queued
authority writer. They do not establish production Store/Session/journal policy
installation, durable operation recovery, public media subscriptions, connection
end, general inactive semantics, reliable-data lifecycle or the full application
workflow. Those remain Gate 6 work. This profile grants no durable operation
receipt and creates no competing journal.

Full Gate 6 still requires Linux x86_64 and the approved remote/native/browser
profiles. Named external/provider rehearsals need the bounded G6-093 configuration
and designated endpoints. Publication remains separate.
