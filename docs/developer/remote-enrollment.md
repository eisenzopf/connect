# Remote client enrollment (G6-055 in progress)

The enrollment protocol types are implemented. Store transactions, signature
verification, HTTP enrollment endpoints and native/browser enrollment clients are
not implemented yet. This checkpoint does not enable a fresh client to obtain a
credential. Existing trusted backend issuance remains described in
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
current ceiling when Store admission is implemented. Browser profiles require an
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
not another body field. Store admission must verify key validity and possession,
current issuer/recipient authority, challenge state/attempts, expiry and durable
one-effect replay before granting anything. Stable normalization remains usable
when inspecting saved original intent; the Store must distinguish a fresh grant
from completed original recovery.

Debug output for request/offer/claim is redacted. Intentional JSON encoding carries
the protected challenge and must stay private. Neither Device-key possession nor
an Endpoint declaration establishes provider consent, route readiness or media.

[Component evidence](../evidence/gate-6/enrollment-contract.json) records six
syntax/binding/normalization tests, including an independently computed canonical
proof vector, in default and schema-enabled builds. Stolen/expired challenge
admission, cryptographic refusal, Store/current-authority replay, protected
physical credential delivery, rotation/revocation, actual browser/native setup
and the server HTTP/OpenAPI reference remain G6-055 implementation work.
