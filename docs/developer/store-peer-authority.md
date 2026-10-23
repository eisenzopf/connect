# Store-backed signed peer authority (G6-037, in progress)

This continuation supplies an explicitly composed StorePeerAuthority for canonical
peers and the installed message.send, message.history and conversation.get
handlers. It follows [provider reconciliation](provider-reconciliation.md) and
keeps schema 31. The [verification report](../evidence/gate-6/store-peer-authority.json)
records the exact source, selected local fixtures and unresolved scope.

Admission checks persisted credential issuance, current principal facts, tenant,
Identity, exact registered Device kind/platform, the signing key's algorithm,
validity and explicit credential-to-key-to-Device binding. Refresh admits both
old and next credentials against that same key before replacing the incarnation.
The union proof and refresh delivery honor both validated source expiries; proof
expiry can only narrow. The successor binding lifetime uses its own renewed
credential and current key, allowing renewal beyond the predecessor deadline.
A key registered to the same Identity alone grants no Device authority. Forged
validated-bearer snapshots and another policy's retained lease cannot register a
binding lifetime.

Each installed data command derives the Conversation and all current member
Identity read scopes, waits outside SQLite, then rederives and checks coverage in
a consistent transaction. The retained typed proof carries its runtime owner,
exact credential, command and Conversation. SQL checks that proof before work and
before committing a send or returning history/snapshot data, including expiry
after waiting for SQLite. Wrong-command, wrong-Conversation, wrong-credential and
foreign-root proofs fail without a fallback acquisition. A held send completes
while actual key retirement waits; it does not reenter a reader behind the queued
writer. Ordinary control operations continue through their existing Store policy.

Unsupported command families and unsupported resource assertions can acquire only
an Identity/key proof for a bounded handler rejection. They receive no resource
permission, cannot pass the installed SQL command checks, and never fall through
to stock Core handlers. The public facts-only invocation carrier cannot authorize
the signed handler without this policy's private typed grant.

The signature verifier retains the original admission lease across verification;
Store key lookup runs in spawn_blocking. The canonical writer carries that lease
through cleanup, caps its deadline by credential/key validity, and checks its
nonblocking current predicate before every poll of the actual send future.
Runtime retirement cancels the component signal synchronously. A bounded weak
registration registry checks current Store facts every 100 ms to retire idle
bindings after credential, key, Device or Identity retirement. These signals only
remove authority: every new command performs fresh admission. Lifetime custody
tracks clones and releases capacity when the final binding lifetime drops. Key
and credential deadlines also wake pending retirement waits directly; they do
not rely on the polling interval or detached per-binding timer tasks. A Sink poll
can perform several underlying writes; per-write transport admission across every
driver is a separate remaining audit and is not inferred from the poll predicate.

Tests cover real Store enrollment and signed dispatch, exact refresh binding,
forged facts, bounded lifetime capacity, real kernel ownership through lease cleanup
and restart refusal by retained old policy/Store handles, SQL proof confusion, queued key retirement,
expiry behind SQLite, original-lease retention across paused asynchronous key
lookup with a queued revocation writer, refresh-incarnation isolation, and actual
length-prefixed loopback TCP peers retiring for
key/Device/credential/Identity/root retirement and key expiry. A gated real TCP
response holds key retirement busy without mutation until its key deadline closes
the socket without emitting the response; the identical retirement then succeeds.
TCP here qualifies
the peer state machine on an isolated reliable socket; TLS/ALPN and production
listener installation require their separate transport checks. Upstream signing,
peer, dispatch and canonical transport regressions are included in the report.

This checkpoint does not install the policy in the production listener, implement
all canonical commands/extensions, complete all source readers or held
task/transition composition, qualify HTTP response-body delivery or replace the
Session model. Voice/SIP and bootstrap/provisioning network custody, all expiry
and bulk profiles, post-retirement attributed settlement, full transport/SDK
parity and native media retirement remain open. G6-020 and G6-037 stay in progress;
G6-038 depends on both. No Linux/remote/browser/live-provider, acceptance or
publication qualification is claimed.
