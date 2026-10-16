# Connect audience and authority contract

Gate 6 target; G6-005. Applies to messages, files, event metadata, transcripts,
media, recordings, exports and derived AI projections.

## Membership and effective access

Identity is tenant-scoped and durable. A Participant is an identity's appearance
in one Conversation. A membership has a monotonically increasing epoch. Leaving
a live Session does not change conversation membership. Removing a conversation
member revokes its current resource access and stops its active resource use.
Re-admission increments the membership epoch; it does not restore access to
earlier private content automatically. Role and kind labels never grant authority.

Effective access requires an authenticated, unexpired, unrevoked principal; an
authorized conversation membership/delegation; current operation/resource scope;
and a matching content audience. All conditions apply to cached results, queued
dispatch, live subscriptions, resumed downloads and active media bindings.
Endpoint ownership and SMS enrollment are independent prerequisites.

## Message audiences

`to` is a nonempty set of Participant IDs or the explicit value `all`. Resolve
`all` into the authorized membership epochs at acceptance, never at later delivery.
Store the sender and effective recipient epochs with the message. No unaddressed
member receives its body, attachment URL, reply reference, receipt metadata or
derived summary. A reply must reference a message visible to its sender and may
not widen the referenced private audience without an explicit authorized sharing
operation. Receipt suppression does not suppress the underlying delivery fact.

A tenant operator is not implicitly a content reader. Administrative access to
content requires an explicit audience-bearing delegation and is audited. An app
may delegate a supervisor/assistant view; a role name alone is insufficient.
History sharing is an explicit permitted action recording source, audience and
provenance. It cannot be implemented by rewriting the original audience silently.

## Uniform projection

| Surface | Required behavior |
|---|---|
| Live message/event delivery | Filter before enqueue; bound the subscriber's queue |
| History/snapshot/search | Select only currently visible items; no leaked private totals |
| Replay/status | Recheck current authority; redact any no-longer-authorized result |
| Files and download links | Audience applies to metadata and bytes; expiry alone is insufficient |
| Receipts and correlations | Emit only to the permitted audience; no private address disclosure |
| AI context | Select first, then bound/summarize; retain source references and omissions |
| Transcripts and vCon | Apply the audience of each source segment, not just Session membership |
| Diagnostics | Redact body, keys, numbers and SDP secrets; sensitive values do not enter Debug |

Authorization errors do not disclose whether an otherwise inaccessible private
resource exists. Ordered cursors may have gaps; gaps never reveal another
audience's event bodies or counts.

## Media and consultation

Session presence, stream publication, subscription and media audience are distinct.
A subscription can select only streams already permitted by audience and authority.
Consultation admits a private human/AI audience while the organizer Connection and
main Session remain alive. The organizer receives only the declared hold treatment.
Private audio never reaches the organizer or a public AI provider leg, including
during preparation, cancellation, handoff, recovery or recording export.

The application decides its consent/business policy. Connect evaluates a configured
policy hook or existing delegated capability and records its decision. A client
cannot grant itself permission by sending `approved: true` or model-generated text.
Ordinary preauthorized agent operations do not require a new prompt or task grant.
