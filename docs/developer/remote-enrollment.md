# Remote client enrollment (G6-055 in progress)

The enrollment protocol and Store transactions are implemented. HTTP enrollment
endpoints, transport-bound credential admission and native/browser enrollment
clients remain pending. This Store checkpoint does not enable a fresh remote
client to obtain a credential over the network. Existing trusted backend issuance remains described in
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
must transfer that owner to physical response custody. Copying the result or
reading `enrollment_credential_binding` facts does not grant later transport I/O.
Schema 115 includes enrollment in snapshot digests and refuses older candidate
state unchanged; retain a matching-binary backup and explicitly initialize fresh
state. No automatic migration is provided.

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
fixtures. Physical credential delivery, trusted transport/Origin admission,
actual browser/native setup and the server HTTP/OpenAPI reference remain G6-055
implementation work.
