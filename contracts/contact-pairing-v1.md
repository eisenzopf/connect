# Native contact pairing in Connect control/1

The additive native operations are carried by the existing authenticated `connect.request` envelope. They require the existing `resolve` authority and permitted recipients. They are ordinary communication operations; optional tasks/grants are not a prerequisite. Task-scoped invocation does not admit these operations because they are independent of a conversation.

| Operation | Inputs | Result |
| --- | --- | --- |
| `contact_pairing_start` | Stable `request_id`, exact `peer_identity_id`, fixed Unix `expires_at` within ten minutes | `contact_pairing` with owner, peer, expiry, pending state and a one-use protected challenge |
| `contact_pairing_complete` | Stable `request_id`, `pairing_id`, challenge presented under the independently authenticated peer identity | Pairing state; no challenge in the response |
| `contact_pairing_revoke` | Stable `request_id`, `pairing_id`, authenticated initiating owner | Revoked pairing state |
| `contact_state` | Exact permitted `peer_identity_id` | Contact native ownership state and verification timestamp; `sms_consent` remains false |

The owner can have at most 32 active pending challenges. Each challenge expires within ten minutes and admits at most five unsuccessful presentations by the intended peer. The database retains a challenge verifier. The initiating receipt contains the intentionally returned challenge; the database and credential-bearing client files are private. Challenge output must not be logged or placed in process arguments; the CLI completion command reads it from standard input.

A wrong identity cannot present a challenge. Both the initiating and completing credentials must have current authority and mutually permitted identity recipients. Completion is one use: another mutation ID cannot re-complete a used challenge. Replaying the same logical request returns its durable receipt only after current resource authority is checked. Revocation of the pairing or initiating credential denies completion replay; revoking the completing credential also makes subsequent contact observations revoked. A new pairing is required to prove possession of a replacement credential.

An unsuccessful permitted attempt returns a pending or attempts-exceeded result and commits its counter atomically; it never produces verified ownership. Expired, revoked, exhausted and unknown states remain distinct. Cancellation is represented by the explicit owner revocation operation. Start retries must preserve the same request ID and expiry; a changed logical intent is rejected by existing mutation receipts.

`verified_native` establishes possession of a current Connect identity credential. It does not establish phone/address ownership, telephone caller identity, conversation membership, route readiness, consent, or SMS enrollment. Pairing creates no conversation and never combines histories. SMS policy evidence continues to come from the configured authenticated enrollment authority and signed provider controls.

Source qualification: `connect-store/tests/contact_pairing.rs` and the provider-free `connect setup demo` native request path. Installed, browser-reference, independent-user and actual-model claims require their own Gate 5 proofs.
