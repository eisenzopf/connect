# Local record retention

The canonical server accepts an installation-owned `record_retention` table in
its protected configuration. Omission selects the named
`connect-local-until-deleted-v1` policy. This changes future terminal Session
intents only. It grants no capture or remote deletion authority.

```toml
[record_retention]
name = "local-metadata-30-days-v1"
metadata = { kind = "after", seconds = 2592000 }
transcript = { kind = "after", seconds = 604800 }
recorded_media = { kind = "after", seconds = 86400 }
provider_reference = { kind = "until-deleted" }
```

Each category explicitly uses `until-deleted` or `after` with one through
315,576,000 seconds. Names contain at most 128 ASCII identifier characters.
Schema 79 freezes the complete policy in the transaction that ends the Session.
Changing configuration or restarting before materialization cannot extend that
original deadline. Deadlines start at the frozen terminal timestamp.

The current producer emits metadata only. Its sources use the metadata limit;
transcript, recorded-media and provider-reference limits are retained with the
intent for an explicit future capture producer. Their presence does not imply
those sources were captured. The canonical manifest retains each source's own
metadata and content deadlines and actual audience. Expired document metadata
is unavailable to discovery even before the worker physically removes its bytes.

The owned `session-records` worker finalizes at most 32 outcomes and processes at
most 32 retention items per iteration. Core selects canonical expiry actions.
Store first commits the exact expected handle, operation ID, cutoff and source
selection, then applies that durable decision. A later retry reuses all four.
A competing record revision supersedes the decision without overwriting newer
bytes; a later iteration selects against that new revision.

Expiry uses the same exclusive record fence as public erasure. Canonical bytes,
mutation receipt, expiry indexes, journal hint and completed job commit together.
Transformation or journal quota failure retains the pending decision and rolls
back every erasure effect. Restart resumes pending work. Replaying completed
work appends no duplicate hint and cannot resurrect a deleted record. Journal
provenance uses an opaque operation ID without a record UUID or content.

Deadlines still apply to authorized projections while worker storage is
unconfirmed. The worker reports degraded readiness on errors. Local expiry
removes local access and does not assert provider deletion, physical disk
sanitization, captured-media coverage or completion of G6-046. Qualification
is recorded separately with the concrete checks that ran.
