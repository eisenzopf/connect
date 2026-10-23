# Explicit user consent and canonical step-up

Schema85 adds immutable `user_delegations` receipts. A currently authenticated
human explicitly authorizes a narrow successor credential through
`POST /v2/user-delegations`; the [HTTP reference](../../contracts/openapi-canonical-http.json)
describes its request, response and failure behavior. `ApplicationClient::issue_user_delegation`
uses this endpoint with an expected consenting Identity and a caller-owned
logical operation ID. Retain the complete request after uncertainty. An exact
retry by the same current issuer returns the same credential and expiry.
Operations and recipients are normalized sets; their ordering and duplicate
entries do not create different consent.

The server derives the consenting user from the authenticated credential. The
request cannot name that user or an assurance label. The issuer must have
`credential.delegate`, every requested operation and recipient, and current
human Identity authority. The target must be current in the same tenant. The
successor receives only the explicit operations and recipients, bounded by the
issuer's expiry. Wildcards, `admin`, `credential.delegate` and consent chains
are refused. A new operation is required to change or renew consent. Receipts
remain reserved, bounded by the configured retained-operation capacity and1023
distinct subjects per consenting user so revocation retains bounded custody.

The successor is a separate credential. Its Device/signing-key binding must be
enrolled explicitly before native authentication or step-up; another credential's
key binding cannot substitute. Canonical authentication returns `user-authorized`
assurance only from the current immutable consent receipt. The authenticated
principal's Identity remains the successor's Identity; the separately recorded
`user_id` is the consenter. Roles, advertised capabilities and ordinary
credential scopes never manufacture this provenance. Ordinary credentials
continue to carry identified assurance.

Protected canonical server TOML can install up to64 `user_assurance` rules:

```toml
[[user_assurance]]
conversation_id = "the-existing-conversation-id"
command = "conversation.close"
user_id = "the-consenting-human-identity-id"
```

Each rule names an exact Conversation, an installed mutating command and its
required user. The required granular operation follows that command, such as
`close` for `conversation.close`. Duplicate or unsupported rules refuse startup.
The policy is immutable for this runtime and is shared by the signed peer and
HTTP facade. Current resource membership, recipient ceilings, revisions, Device,
key and both old/new credentials remain separate Store requirements.

A signed request with insufficient assurance suspends its original logical
mutation and receives `identity.step-up-request`. Use the SDK's explicit
[`complete_step_up`](step-up.md) exchange. It answers the cached challenge on
the same physical peer without resending the original command. The server
revalidates both credentials, the exact old/new key binding and unchanged
Identity/Device, the configured user and operation, and original resource
authority before resuming the held request. The original operation ID, expected
revision and deadline are preserved. A successful authentication transition
still requires inspection of the original business outcome.

HTTP commands enforce the same floor and do not initiate a native challenge.
Supply an already current consented credential to perform a protected HTTP
command. An ordinary identified credential cannot bypass the floor through HTTP.
The SDK can reconnect with a current user-authorized credential; refresh
continues to preserve existing assurance provenance rather than elevate it.

Every credential entry point revalidates the consent's current issuer, user,
tenant, operation/recipient ceiling and expiry. Revoking the consenting
credential or Identity acquires write custody for affected successor subjects
before mutation, so existing delegated IO readers stop first. Future uses and
receipt replay refuse revoked consent. Independently issued credentials remain
independent. Consent does not make the user a Conversation Participant or expand
recording, consultation or other content audiences.

The [component evidence](../evidence/gate-6/auxiliary-user-assurance.json) records
the actual checks and preserved failures under G6-046. Canonical task-scoped
grant installation, owned-audio assurance transitions, exact auxiliary vectors,
Linux/browser/remote/provider profiles and acceptance remain separate required
work. No upstream source patch or automatic schema downgrade is introduced.


The [native step-up continuation](../evidence/gate-6/native-media-step-up.json)
qualifies owned native DTMF for real current user consent and the same enrolled
Device/key, with explicit full native operation scopes. It refuses unenrolled
consent before media effects, preserves the held envelope and logical mutation,
and separates successful authentication from original SQL failure/retry. Both
signing algorithms pass. The
[signal-only evidence](../evidence/gate-6/native-signal-authority.json) additionally
qualifies consent with only `dtmf.send`, current physical credential rotation,
continued actual signaling and refused new RTP publication. A separately enrolled
receive-only credential can refresh and receive a new signal on its original
physical peer. Task successors, production partial/recovery profiles, exact
auxiliary clauses and acceptance remain open.
