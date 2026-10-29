# Remote client enrollment (G6-055 in progress)

The enrollment protocol, Store transactions, bounded HTTP endpoints and
credential-specific admission are implemented. The public Rust native enrollment
workflow is described in [native enrollment](native-enrollment.md). Browser
installation and complete end-to-end qualification remain pending. Existing
trusted backend issuance remains described in
[credential installation](credential-installation.md).

`connect_protocol::enrollment` defines a backend-approved `EnrollmentRequest`,
a protected `EnrollmentOffer`, and the client's `EnrollmentClaim`. The approval
binds the target Identity, Device, key, native/browser transport, Endpoint
address/capabilities, literal operation/recipient scopes, exact browser Origin,
absolute expiries and original approval/completion operation IDs. Provider and
operator secrets are not request fields. The existing provisioning and scoped
issuance validators normalize addresses and permission sets; normalization never
grants authority.

Fresh challenges expire within ten minutes. Fresh browser credentials are
limited to one hour and native credentials to one day, subject to the issuer's
current ceiling at Store admission. Browser profiles require an
exact serialized HTTPS Origin. QUIC accepts no browser Origin. Native endpoint
URLs retain the existing explicit-port rule. A Device kind is descriptive; it
never grants a role, ownership or reachability.

The protected offer includes the installation, issuer Identity, enrollment and
Endpoint IDs and a 32-byte challenge encoded as 64 lowercase hexadecimal
characters. `offer.proof_message()` produces:

```
ASCII("connect-enrollment-proof/1") || 0x00 || SHA256(JCS(normalized offer))
```

Sign those exact 59 bytes with the approved client-owned Ed25519 key, or with
ES256 using SHA-256 and a raw 64-byte `r || s` signature. The claim uses canonical
unpadded base64url for either signature. No private key leaves the client.
The proof commits every approved field, including the challenge; changing an
Endpoint, key, Identity, issuer, scope, expiry or original logical ID changes it.
Equivalent normalized set ordering retains the same proof context.

`claim.check_binding(offer, actual_origin)` checks bounded signature syntax and
the original claim/Origin bindings only. It does **not** verify a signature or
issue authority. The actual Origin must come from trusted transport context,
not another body field. Store admission verifies actual curve/key validity and
Ed25519 or ES256 possession, current issuer/recipient authority, challenge
state/attempts, expiry and durable one-effect replay. Stable normalization remains
usable when inspecting saved original intent; fresh admission and completed
original recovery have separate expiry checks.

`Store::approve_enrollment` requires a current administrator and reserves both
original operation IDs. Approval creates no Device, Endpoint, signing key or
credential. At most 32 unexpired pending approvals exist per target Identity.
The offer challenge is deterministically derived with an installation secret;
only its hash is stored. Challenge knowledge alone cannot complete enrollment.
Five matching but cryptographically invalid claims exhaust a pending approval.
Wrong target, transport, Origin or challenge claims make no changes.

`Store::claim_enrollment` atomically creates the scoped credential, Device,
unverified Endpoint and exact credential/key binding. Any late conflict rolls
back the whole transaction. A completed signed retry recovers the same bearer
and absolute expiry, including after the challenge expires, while the original
issuer, child credential, Device, key, Endpoint, recipients and original receipt
remain authorized. The signature bytes are not the immutable intent; an ES256
client may sign the original context again. Receipts contain IDs and timestamps,
never bearer or raw challenge material.

Rotation uses fresh approval/completion IDs and a fresh key, and can reuse the
exact active Endpoint/Device. `Store::revoke_enrollment` requires current backend
administration and revokes that approval, credential and key. It retains shared
Device/Endpoint resources so a replacement can continue. Independent Device or
Endpoint revocation still prevents recovery. Another currently authorized backend
can revoke even when the original issuer is no longer active.

Store results retain authority fences through `OwnedEnrollment`; an HTTP adapter
transfers that owner to physical response custody. Copying the result or
reading `enrollment_credential_binding` facts does not grant later transport I/O.
Schema 115 includes enrollment in snapshot digests and refuses older candidate
state unchanged; retain a matching-binary backup and explicitly initialize fresh
state. No automatic migration is provided.

The [HTTP/OpenAPI reference](../../contracts/openapi-enrollment.json) describes:

- `POST /v2/enrollments` on the administration surface: authenticate the current
  ordinary administrator before parsing the complete original approval request.
- `POST /v2/enrollments/{id}/claim` on the application surface: send the signed
  claim without a bearer. The path and body enrollment ID must agree. Browser
  claims send their actual approved Origin; native QUIC claims send no Origin.
- `POST /v2/enrollments/{id}/revoke` on the administration surface: send
  `{"operation_id":"original-revoke-id"}` with current backend administration.

Remote use requires explicit HTTPS hosting and trusted certificates. The same
routes are available on the existing loopback HTTP listener for local use.
Administration rejects every Origin; claim rejects every Authorization header,
including an operator bearer. Host, provider identity headers, duplicate Origin,
query parameters and HTTP version are checked before parsing. HTTPS hosting also
requires the Origin to be on that application's configured allowlist. The claim
checks the exact approved Origin even if another Origin is on that allowlist.
No bearer or challenge belongs in a URL.
Listener policy can reject an invalid Host or unlisted Origin earlier with the
canonical `profile: "connect/2"` problem response; route rejections use
`profile: "connect-enrollment/1"` and a typed `result.Err`.

All three routes bound JSON to 64 KiB and share one five-second admission,
body-collection, Store and physical-response deadline. Responses use closing
HTTP/1.1 with `Cache-Control: no-store` and `Pragma: no-cache`. The actual socket
retains Store authority and shared capacity after Hyper consumes body bytes.
Cancellation or the earliest issuer/credential/admission expiry stops protected
writes. Detached blocking Store work retains its capacity slot until it ends.
A transaction can commit after response observation ends: preserve the original
offer, request, logical IDs and client private key, then explicitly reconcile.
Neither a timeout nor a lost response authorizes a replacement grant.

The actual accepting QUIC, WebTransport or WSS driver now supplies the physical
transport and, for browser transports, its exact accepted HTTP Origin to the
handler. The Origin survives the upgrade in a trusted Rust context. It is not
read from a signed envelope or inferred from an allowlist. Under the original
verified credential/Device/key lease, the handler checks this context against
the enrollment before producing `auth.session`, and before installing a refresh
or step-up successor. The reliable-data wrapper forwards this admission hook.
A separately listener-allowed Origin still fails the credential's exact binding.

Current metadata and peer proofs recheck the enrolled child, key, Device,
Endpoint and immutable receipt. A second active key attached to the credential
cannot replace its approved key ID. Endpoint revocation therefore refuses fresh
current admission. This does not change the earlier distinction between network
readiness, provider consent and a signed enrollment proof.

Finite application HTTP keeps its own operation and current-authority admission
and additionally checks the enrollment's exact Origin, including native absence.
Explicit Task bearers retain the enrolled backing credential's Origin ceiling;
they cannot bypass it by presenting their separate narrow token. Local MCP
admission applies the same check. Ingress with no trusted physical enrollment
context refuses enrolled credentials; the dedicated Origin-free record HTTPS
listener also refuses browser-enrolled credentials. These checks add no scope.

Key rotation uses a fresh approved enrollment and a fresh correctly bound peer.
Retiring the old enrollment leaves the shared Device/Endpoint available to the
replacement. A fresh key is not silently substituted through an old signed
credential-refresh envelope. Ordinary independently issued credentials retain
their existing refresh and assurance behavior.

Debug output for request/offer/claim is redacted. Intentional JSON encoding carries
the protected challenge and must stay private. Neither Device-key possession nor
an Endpoint declaration establishes provider consent, route readiness or media.

[Protocol component evidence](../evidence/gate-6/enrollment-contract.json) records six
syntax/binding/normalization tests, including an independently computed canonical
proof vector, in default and schema-enabled builds.
[Store evidence](../evidence/gate-6/enrollment-store.json) adds 13 local behavior
functions, including both algorithms across all three transport intents,
stolen/expired challenges, current-authority recovery, revocation, rotation,
atomic rollback and protected in-process delivery. These are isolated SQLite
fixtures. [HTTP evidence](../evidence/gate-6/enrollment-http.json) records three
local behavior functions, six algorithm/transport-intent scenes and 36 captured
bodies checked against the generated reference.
[Physical-admission evidence](../evidence/gate-6/enrollment-admission.json) records
the corrected regressions and actual local signed QUIC/WT/WSS admission; these
earlier HTTP claims do not qualify later connections.
Public browser enrollment setup and complete G6-055 qualification remain
implementation work.
