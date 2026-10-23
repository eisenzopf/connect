# Durable canonical peer credentials (G6-037, in progress)

Fresh state is schema 31. Schema 30 and earlier candidates are preserved and
refused unchanged; initialize a new explicit state path. There is no automatic
conversion, inferred key enrollment or invented issue timestamp. A new credential
stores its actual issuer time in the same insert as its verifier and scope.
`StoreValidator::canonical().validate_credential` returns that immutable time and
its persisted credential ID. Neither fact is inferred from validation time.
The principal subject is the stable Identity ID. The specific credential ID is
carried in `ValidatedBearer.token_id`, so rotation does not change the peer owner.
`StoreCredentialBinding` validates those issuer facts and explicit key bindings;
the installed policy carries them alongside its owned fence in
`StoreInvocationAuthority`. The canonical handler obtains its credential from
that trusted invocation context and revalidates it before calling the Store.

The trusted admin HTTP API adds these operations to the existing bounded
provisioning service and durable receipt ledger:

| Route | Body type | Effect |
|---|---|---|
| `POST /v2/identities/{id}/signing-keys` | `signing_key` | Enroll an Identity/Device verification key and explicitly bind a current credential |
| `POST /v2/signing-keys/{id}/revoke` | `revoke_signing_key` | Permanently revoke that key at its current revision |
| `POST /v2/devices/{id}/revoke` | `revoke_device` | Permanently revoke the Device and retire its endpoint records |

See the [HTTP request/response reference](../../contracts/openapi-provisioning.json).
The middleware requires a current admin credential before body decoding, refuses
browser Origin and provider identity headers, and applies the existing 64 KiB
body, concurrency and five-second total request bounds. Body and URL targets must
match. Tenant/recipient/owner checks and mutation receipts share one transaction.
Retry the same logical operation ID after uncertain transport loss. Replaying a
receipt returns the current key/device metadata and cannot restore revoked state.

Enrollment accepts EdDSA (32-byte Ed25519 public keys) or ES256 (65-byte
uncompressed SEC1 P-256 points), encoded as canonical unpadded base64url.
The Store rejects malformed points, weak Ed25519 keys and invalid P-256 points.
Public key material, owner, Device, algorithm and validity window are immutable.
The expiry must be in the next year; future `not_before` is allowed and prevents
resolution until that time. A new key uses `expected_revision: null` or omits it.
Binding another credential to an existing key requires its current revision and
the exact original key metadata. Adding a new binding advances the key revision.
A new credential for the same Identity cannot borrow a previous binding.

History is bounded without eviction: at most 64 key records per Device and 128
credential bindings per key, including retired history. Overflow refuses the
new grant while preserving existing bindings. Revocation is irreversible; a new
key ID is required for later enrollment. SQL ownership constraints prevent
cross-owner bindings and immutable owner/issuance/key facts cannot be rewritten.
Snapshots preserve issuance, key material and binding rows in their logical
digest; restoring one does not mint new issue times or verification keys.

`StoreKeyResolver` has no global or context-only fallback. It checks the local
issuer, installation namespace, stable Identity owner, active Device, algorithm
and key validity. This establishes cryptographic ownership. The peer policy must
separately check the exact credential/key binding through `StoreCredentialBinding`
while holding its authority fence; an Identity-owned signature alone grants no
credential permission. The handler also rechecks the current backing credential.
The authenticated principal's tenant namespace is the installation ID, while the
key registry also enforces the stored tenant/Identity relations. These checks
remain distinct to prevent installations with the same default tenant name from
sharing authentication authority. Exact binding resolution rejects revoked credentials. Identity-level lookup
rejects retired Identities, Devices and keys; it never grants credential permission.

A lookup or credential binding returns facts, not an owned execution/delivery grant.
Production peer policy must still validate the signed key's Device against the
claimed peer Device, retain current authority across effects and writer delivery,
and install retirement watchers across every authority-changing facade and Store
process. Native media cleanup and current endpoint/session projection retain
their integration tasks. The runtime still installs the previous native handler;
the production canonical listener, full command dispatch and QUIC/HTTP/SDK parity
remain open. No provider, remote host, browser or Gate 6 acceptance is qualified
by these credential records.
