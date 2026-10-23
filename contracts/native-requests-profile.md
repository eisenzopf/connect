# Native server request origins

A subscription to a participant includes future streams. Preparing a later
receiver binding can require a new server `connection.offer` after the original
subscribe handler has ended. The server must obtain fresh current authority for
that work. It must not retain the original invocation's grant or fabricate a new
`AuthorizedEnvelope` from stored request metadata.

`CanonicalNativeAdapter.native_request_origin` seals an idle origin only from an
actual successful signed native negotiation on the same physical route. It binds
the configured Conversation/Session/Connection wire scope, current credential,
member epoch, endpoint/device, controller and physical incarnation. The origin
holds metadata and policy/verifier interfaces, with a weak reference to the
original grant. It becomes usable only after that original grant is released.
Its lifetime follows the credential and physical route, rather than the completed
request's shorter dispatch deadline.

Every request requires `NativeRequestPolicy.authorize` for the exact current
operation and proposed parameters. The scope includes the actual internal
Conversation, connection generation, credential revision and expected stream
revision. An explicit held variant lets an already-running server mutation reuse
its freshly acquired guard under the same domain and intent, without reacquiring
storage behind a queued writer. An origin is never that grant.

## Actor, custody and response

Requests are restricted to scoped native `connection.offer` frames with a logical
operation ID, expected revision and the actual physical substrate. A shared
negotiation slot covers server requests, incoming offers and codec updates on the
same native route. It stays owned through the active reply's mutation custody.
The physical actor owns bounded queuing, current signature/key approval, replay
checks and exact correlated reply validation. No detached writer is introduced.

Before dequeue and every IO poll, the request must still match the physical
incarnation, credential, controller revision, confirmation epoch and current
grant. Authority cancellation or a deadline aborts pending IO. The actual sink is
destroyed before its grant is released if IO may already have started. A stale
queued request that has not reached IO preserves the physical writer.

`NativeRequestReply` contains the authenticated response, proposed parameters and
actual current policy guard for a bounded checked mutation. The actor replaces
the proposal's envelope ID and timestamp; the response's `in_reply_to` identifies
the actual signed wire request. The proposal accessor is explicitly the template,
not a claim that its template ID was transmitted. The reply must be dropped when
the mutation completes; it is not an idle cache or a durable operation receipt.

The caller still verifies exact endpoint parameters against its real provisional
resources and commits through the media/route transaction under current authority.
A signed correlated answer alone is not readiness, Participant presence, a media
commit or a durable business result. This transport path does not itself allocate
receiver resources or update subscription tables.

## Failure and uncertainty

Scope mismatch, stale revision, denied authority, cancellation before submission
and queue refusal do no request IO. Once IO has been attempted, cancellation,
retirement, expiry or a lost answer reports `SubmissionUnknown`. Preserve the
logical operation ID and reconcile journal/current resource state before retry.
A late answer cannot revive a retired exchange. Invalid reply scope/signature
cannot consume a subsequent valid correlated answer.

The component checks use isolated QUIC/WebTransport peers and controlled current
domain policy. They cover a request after its source grant's deadline, idle reader
release, fresh denial, held grant under a queued writer, negotiation serialization,
late answers, cancellation and EOF, plus actual writer custody. They do not close
public subscription selectors, atomic media/route resource commits, production
Store/Session/journal integration, automatic refresh or the full application
workflow. The new request path is a prerequisite for those operations, not a
substitute for their implementation or evidence.

Full Gate 6 retains approved Linux x86_64 and remote/native/browser requirements.
Named external/provider rehearsals need bounded G6-093 configuration and designated
test endpoints. Publication remains separate.
