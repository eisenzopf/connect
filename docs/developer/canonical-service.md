# Canonical durable service integration (G6-037, in progress)

This guide records the initial data-handler slice. Current continuations are
[operation recovery](canonical-operations.md), [shared receipts](control-receipts.md),
[schema-33 conversation create/list/close](conversation-control.md), and
[schema-34 membership control](membership-control.md), and
[canonical subscriptions](canonical-subscriptions.md), and
[client read reports and receipt hints](canonical-receipts.md). The
creation/listing and operation gaps described for this initial slice below are
addressed by those checkpoints, including explicit physical live delivery; production listener and full command installation
remain open.

`connect_service::CanonicalHandler` is the installed-handler building block for
canonical `message.send`, `message.history` and `conversation.get`. A canonical
authenticated dispatcher supplies its verified principal and owned current
authority lease. The handler revalidates the local issuer, installation, method,
credential expiry and protocol scopes before calling the ordinary durable Store
APIs. A request's `from` assertion is checked in the message acceptance
transaction. Stable operation/message IDs, acceptance receipts, current
membership and audience filtering therefore share the existing control path.

`StoreValidator::canonical` maps a local bearer to identified bearer assurance.
An ordinary credential does not manufacture a task delegation. Its credential
validator now reads the persisted issuer token ID and original issue time.
Administrator-bound device keys are resolved by `StoreKeyResolver`; see
[peer credential records](peer-credentials.md). These records do not install an
owned authority retirement policy or the production canonical listener.

History returns chronological `message.send` observations followed by a final
`ack` with an optional `next_cursor`. These observations have their request's
`in_reply_to`; they do not invent mutation IDs. Correlated incoming envelopes
can only reach matching authenticated waiters and cannot enter command dispatch.
Resource identifiers remain bounded to 128 bytes. Opaque history, event and
snapshot tokens have a separate 2048-byte bound; Store validation still binds
them to the installation, identity, resource and current visibility.

One invocation can return at most 128 application envelopes and 2 MiB of encoded
application replies. Signing rechecks the byte budget. History pages reserve
one envelope for the final ACK and use at most 127 messages (the Store's
configured page limit also applies). The authenticated challenge advertises
the batch bounds. Every reply gets a fresh signed envelope ID and the original
request correlation. One retained current lease covers the entire ordered
writer submission. Step-up emits its session/assurance prefix and then the
bounded application sequence under the transition lease. Overflow fails;
frames are never silently truncated.

This first handler slice rejects unsupported families, session/connection
assertions, revision preconditions and non-ordered message reliability. It does
not fall through to the stock Core handler. Conversation creation/listing needs
truthful persisted options and opening timestamps. Generic operation commands
need target-bound protocol floors and transactional revision checks. Membership
commands needed invitation/result semantics; live subscriptions needed retained
delivery and current audience authorization. The linked continuations implement
those selected boundaries; production installation and other command families remain open.

The server runtime still installs its previous native handler. Replacing it
requires the production current-key/device/credential policy and retirement
fences across all authority-changing facades, followed by canonical QUIC,
HTTP and SDK parity checks. The local dispatcher fixtures use real signatures
and an isolated durable Store with a controlled transport lease. The upstream
batch fixtures use real pinned local TLS/QUIC and a queued grant writer. Neither
fixture policy qualifies the missing production installation, providers,
remote hosts, browser profiles or any Gate 6 acceptance criterion.

The [signed native runtime](canonical-runtime.md) now provides an explicit
`serve-canonical` entry point and a public signed Rust transport. It installs
this service and its physical subscription factory in the actual listener.
The default runtime and high-level SDK migration remain open.
