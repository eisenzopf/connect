# Host context projection v1

`DeliveryRequest::context` is either absent or one of the following JSON envelopes.
The public `connect_connector_api::host_context::HostContext::decode` checks a byte
bound before parsing and supplies API-only typed DTOs. It selects no history and
performs no authorization. Connect has already authorized and journaled the exact
selection; adapters must retain it separately from current `body` and must not
fetch more history or reuse remote history to expand it.

## Ordinary conversation projection

```json
{
  "conversation_id": "conversation-one",
  "watermark": 7,
  "items": [
    {
      "sequence": 7,
      "kind": "message",
      "source_id": "message-one",
      "actor_id": "human-one",
      "body": "The approved color is violet."
    }
  ],
  "omitted": 0,
  "deliveries": [],
  "deliveries_omitted": 0
}
```

`conversation_id`, `watermark`, `items` and `omitted` are required. `deliveries`
and `deliveries_omitted` default to empty/zero when absent for older projections.
`watermark` and each `sequence` are unsigned journal positions; `omitted` counts
eligible source records excluded from this bounded selection. Items are ordered
oldest first within the selected recent page. The same `source_id` can occur at
multiple journal positions: a Session has separate invite, accept and end events.
The mapping therefore identifies a selected item as `journal:<sequence>:<source_id>`
and also retains its original source ID in provenance.

Each item requires `sequence`, `kind`, `source_id`, `actor_id`, `body`. Source IDs
refer to the original message, session or artifact. Authors are the message sender,
session initiator or artifact owner, respectively. Body is unchanged message text
or serialized session/artifact metadata; it is never artifact bytes or a server
path. In this ordinary projection, session/artifact metadata is the saved event
snapshot. No item or external reference grants permission.

Ready canonical records use `kind: "record_ready"`; a later explicit share can
produce `record_history_shared`. Their `source_id` is the record UUID. Each carries
an optional `record` reference with `uuid`, `session_id`, `revision`, `state`,
`content_hash`, and `valid_until`. State is `available`, `unavailable`, or `deleted`.
An available body is the unsigned canonical projection for the current owner,
delegate, and actual listener audience. Its hash covers only that projection.
Unavailable and deleted rows retain an empty body and a null hash. Neither the
whole-record hash, manifest hash, storage locator, nor source URLs are exported.
The reference grants no access; `record_get` is required independently of
`context_view`, including each task and invocation-grant ceiling. Record history
sharing preserves the original private Connection audience restrictions.

`valid_until` is the earliest visible retention boundary, or null. The host
shortens actual transport custody to that boundary and compares the original
selection against current authorization and record content before export or
opaque-memory reuse. Adapters must retain the reference in provenance and must
not resolve it to a broader record.

Canonical Session outcomes use `kind: "session_control"`. Their body is the
serialized `session_control::SessionView` from that exact journal event, including
its observed state and revision. A later accept/end or server restart does not
rewrite an earlier invite outcome. `actor_id` attributes the Session initiator
who owns the source; it does not assert who performed every subsequent action.
The host applies both current principals and the destination audience before
selection limits and omitted counts. Delivery admission binds the original
journal kind, source ID and body; replacing a historical body with current
metadata or relabeling its event is refused.

A public native assistant export is bound to its actual Connection generation,
verified provider Endpoint and every current Session listener. A private export
is bound to its original private purpose and every original private listener.
Both selection and later opaque-memory use must check every source against that
whole audience. When the context owner and assistant delegate are the same AI,
that identity cannot substitute for another listener's source permission.

A delivery entry has fields `id`, `session_id` (string/null), `provider`,
`watermark`, `state`, `provider_ref` (string/null), `created_at`, `updated_at`.
Native control receipts additionally carry nullable `destination`, `problem` and
`lifecycle` fields. `destination` contains the original `connection_id`,
`generation`, and `purpose` (`public_session` or `private_consultation`). Retirement
or a later generation does not rewrite that binding. Private generations are read
from the saved private selection, never the current mutable Connection row.
`problem` is `rejected`, `truncated`, `incomplete` or `unknown` when observed.
`lifecycle` records `creation_admitted`, `creation_confirmed`, `export_admitted`,
`delivery_confirmed`, `closed`, and nullable `cleanup_admitted`/`cleanup_uncertain`.
Its `phase` is `initialization`, `post_creation` or `update`; nullable
`parent_delivery_id` binds an update to the original captured instance. Update
children admit no creation and may confirm the already captured instance.
Their cleanup fields describe that parent instance, including its later closure.
An initialization export is admitted before its creation response, so an uncertain
receipt with no provider reference may still describe history sent inline.
Null cleanup fields mean that lane records no separate admitted cleanup phase.
These facts describe control observations, including terminal status without an
end packet; they do not establish audio readiness. Ordinary unbound receipts have
null native metadata. The [HTTP reference](../openapi-context.json) describes the
current named application read API. Times are Unix seconds. Receipts are observations of exported selections, not
additional model-visible history or credentials. `deliveries_omitted` counts older
receipts outside this recent list; authorized public receipt pagination is separate.

## Task-origin recall projection

A task-origin delivery with history permission instead carries the existing tagged
public `TaskOutcome::Recall` result, not `ContextView`:

```json
{
  "kind": "recall",
  "data": {
    "sources": [
      {
        "source_id": "message-one",
        "kind": "message",
        "actor_id": "human-one",
        "created_at": null,
        "provenance": "conversation-journal:event-one;timestamp-not-recorded",
        "state": "available",
        "body": "The approved color is violet."
      }
    ],
    "next": "opaque-authorized-cursor",
    "omitted": 2
  }
}
```

The producer emits all shown fields. `sources` and `omitted` are required;
missing `next` decodes to None. Source string fields are required; missing nullable
`created_at`/`body` decode to None, retaining their absence. `created_at` may be null because older message
storage has no timestamp; do not invent a query/task timestamp. `next` is a
string/null checkpoint, never permission or an instruction to fetch automatically.
`omitted` counts eligible later entries absent from this page. Source order is
oldest first. Task session/artifact bodies are current authorized metadata, and
provenance explicitly records `current-session-metadata` or
`current-artifact-metadata;bytes-require-artifact-get`, rather than claiming the
saved event snapshot.

Record task sources carry the same optional `record` reference. Their provenance
includes `current-authorized-record-projection`; missing bodies remain null.
The strict body-bearing conversion refuses unavailable/deleted task sources.

`state` is `available`, `deleted`, `expired` or `unavailable`. Missing bodies are
null; unavailable attribution may have empty `actor_id`. Retain these absences.
The generic body-bearing `messaging_items()` conversion refuses unavailable,
unattributed or missing-body task rows instead of inventing an author/body or
silently dropping them. Adapters supporting references without bodies can consume
the typed envelope directly. Empty task recall is valid and remains distinguishable
from no permission/context; the DTO retains omission/cursor metadata even when
there are no body-bearing items. Matching current task authorization remains the
host's responsibility, including revocation and replay.

## Event kinds and derived material

`kind` is the journal event category, not a model role or a free-form summary label.
Current categories include `message`, `session_invited`, `session_accepted`,
`session_ready`, `session_rejected`, `session_cancelled`, `session_interrupted`,
`session_ended`, `artifact_started`, `artifact_ready`, `artifact_deleted`,
`voice_transcript`, `voice_context_status`, `voice_outcome`, and provider observation
corrections ending in `_correction`. Preserve unknown categories and their bodies
as opaque original records; do not guess their meaning from text.

The shared conversion's explicit rule sets `derived=true` only for
`voice_outcome` and `voice_outcome_correction`, whose retained provider end reports
may contain provider-generated analysis/summary. It does not infer derived status
from arbitrary strings such as `summary` or `incomplete-transcript`. Other copied
original records have `derived=false`; this does not assert factual truth or full
transcript completeness. A provider-generated synthesis must separately be marked
derived and identify its actual selected sources.

`voice_transcript` is one final provider excerpt, not a full call transcript.
`voice_context_status` bodies retain explicit incomplete state/missing fields.
`voice_outcome` bodies preserve bounded end-report status, reason, summary and
success evaluation. Correction bodies retain `corrects_source_id` plus the new
body. The decoder leaves all bodies unchanged, including these markers; neither
an incomplete transcript nor derived summary becomes a system instruction.

The conversion serializes source kind, original source ID and omission metadata
into `ContextItem::source`; ordinary rows also retain sequence/watermark, and task
rows retain state, timestamp and exact provenance. Task journal references come
from `conversation-journal:<event-id>` before any semicolon notes. Keep the full
original host envelope alongside the logical mutation ID, including delivery
receipts and cursors, so replay checks compare the actual selection. Current input
stays separate. Reusing an ID with changed input or envelope must be rejected.

The typed v1 parser rejects malformed, oversized, unknown-field or non-context
results before any effect. This is a declared profile boundary, not a compatibility
fallback to unbounded or differently authorized history.
