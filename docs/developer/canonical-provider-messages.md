# Canonical provider messages

The canonical server can own Vapi chat delivery, Telnyx SMS delivery and a
separate verified Telnyx callback listener on its existing Store and service.
Native signed messages and canonical HTTP use the same durable messages, explicit
audiences, context selections, provider jobs and original mutation receipts.
The application does not start a provider worker or invoke an application task
or handoff command to send a message.

A trusted operator configures the existing immutable connector routes, provisions
and verifies their Endpoints, then selects their exact accounts in protected
server TOML. Public callers cannot install a route, choose a private key or
supply their own endpoint ownership through a message payload. Route and
Endpoint IDs below refer to already-created administrative state.

```toml
[provider_messages]
operator_credential_id = "current-operator-credential"

[[provider_messages.accounts]]
connector = "vapi"
secret_reference = "VAPI_CHAT_ACCOUNT"

[[provider_messages.endpoints]]
route_id = "assistant-chat"
connector = "vapi"
endpoint_id = "verified-assistant-chat-endpoint"
account_credential_id = "current-account-owner-credential"
secret_reference = "VAPI_CHAT_ACCOUNT"
```

Select at most one account per connector in this profile. The operator and
selected account credentials must belong to the same current account owner. The selected reference
is resolved exactly through the configured secret-reference store, or its
explicit development environment reference. Missing references do not substitute
another account's key. The Vapi adapter reports its actual private-key reference;
the Telnyx adapter likewise reports its actual API-key reference.

The registered binding freezes the route destination/delegated credential,
verified Endpoint revision/address/capabilities, active Device facts, intended
recipient Participant/epoch and account owner. Vapi chat requires an AI identity
and a verified `vapi-chat` Endpoint with `messaging` capability. The Endpoint
secret reference and provisioning owner must match the selected account. Changes
require a new explicit route binding; changing a role or readmitting an identity
cannot silently inherit the original binding. Enable/disable retains the
existing route policy.

SMS additionally requires a verified `sms` source Endpoint owned by the selected
account owner, with the route's exact source number and key reference. The target
has its own verified `sms` Endpoint. SMS enrollment and the exact
phone/source/Conversation consent tuple remain separate prerequisites. Production
startup expires stale SMS authorities and synchronizes the configured enrollment
policy; absent, expired or denied enrollment cannot authorize a send. STOP/START
controls retain the existing signed-provider ordering and fail-closed policy.
Caller numbers route configured contacts; they grant no new principal or member.

The worker claims only mounted route IDs. It rechecks current Endpoint, Device,
account and membership facts before selecting context and before transport
admission. The actual adapter configuration must match the stored account
reference. Its bounded transport permit includes the current account expiry and
identity fences. A withdrawn Endpoint or account cancels unsubmitted work rather
than exporting context or choosing another key. Context remains bounded by both
the original sender and intended AI's current permissions. Private fanout uses
the original audience and provider replies address the original sender.

Discovery mounts only these message routes and references. Configuration does
not create a reachability observation or promise delivery: unknown reachability
requires the caller's explicit attempt policy. SMS consent remains checked at
selection and dispatch. Pending or uncertain delivery remains distinct from the
local message acceptance receipt. An uncertain send is never resubmitted to
learn its outcome; restart and public replay preserve the original logical IDs.

For a Telnyx account, configure the separate callback surface:

```toml
[provider_messages.telnyx_callbacks]
bind = "127.0.0.1:8087"
verification_key = "base64-encoded-Telnyx-Ed25519-public-key"
```

This listener exposes only `POST /webhooks/telnyx`, with the existing exact raw
body signature/timestamp verification, event identity/digest checks and bounded
admission, scoped to the selected SMS routes. An unknown final provider ID
requires an attributable signed source/destination pair before it can be retained
for reconciliation after the send response. A callback cannot import an unmounted
route. Signed controls from unknown contacts to an owned mounted source still
record opt-out without creating enrollment or a message. Delivery-attempt metadata is not event identity. Tampered or stale
signatures refuse before effects; conflicting replay refuses. Incoming messages
recheck bound Endpoint/account/epoch policy before duplicate handling. STOP/START
and authenticated final delivery facts retain their existing enrollment and
historical effect semantics. The callback port exposes no control or application
APIs. See the [callback OpenAPI](../../contracts/openapi-provider-observations.json)
and [canonical HTTP control](canonical-http.md).

Shutdown closes callback intake and joins the owned delivery worker before
retiring canonical peers. Cancellation records admitted uncertainty and retains
original IDs; it does not claim provider absence. A protected driver injection
boundary supports isolated actual-SDK fixtures with the same account and
Endpoint checks. Local fixtures do not qualify actual provider accounts, remote
hosting or publication; those use the plan's bounded designated rehearsals.
