# Connect records, vCon and model context

G6-009. Every ended Session produces the baseline-required vCon/outcome record,
even with audio capture disabled. Metadata-only records never imply recorded
audio exists. Recording and retention policy are explicit installation/app policy;
they are not inferred from Session membership or an assistant prompt.

## Record projection

The canonical record contains version, Session/Conversation linkage in the local
index, participant appearances, Connection lineage, permitted event/dialog
references, transcript completeness, capture provenance and content integrity.
Use the current rvoip vCon schema and canonical validation fixtures from the
frozen source. Portable Conversation federation linkage remains reserved; do not
repurpose vCon's reserved core group field.

`recording.vcon-ready` contains a bounded authorized handle. Fetch checks current
authority and each source segment's audience. A shared Session can contain a
private consultation, so its members cannot all fetch an unfiltered full record.
Produce a caller-authorized projection; its digest names those exact projected
bytes. Do not advertise the canonical full-record digest for a redacted view.
Media URLs require current authorization and expire; a secret URL alone does not
grant content authority. Small records may be returned inline; large records
use authorized retrieval. A missing or failed record store is explicit.

## Retention and redaction

Apply installation limits and named policy independently to metadata, transcript,
recorded bytes and external-provider references. Default disabled media capture
still creates the Session outcome record. Redaction/deletion has its own durable
operation and expected record revision; concurrent attempts conflict. Remove
access to deleted bytes and invalidate outstanding handles. Preserve an
audience-safe tombstone and explain unavailable source material in projections.
Deleting local data cannot imply deletion by a provider; record the actual remote
deletion result or uncertainty.

## Context delivery

Select permitted source items first, then summarize/bound the projection. Include
source IDs/revisions, watermark, omissions, transcript completeness and applied
audience. Record prepared, delivered, rejected or unknown context delivery to the
actual provider session. A matching Conversation ID is not proof a model saw it.

Some realtime providers consume instructions only at initialization. Create a
fresh private provider leg when required, retaining logical AI identity. Never
copy private consultation into the public provider's persistent chat memory.
The post-session result is attributed to actual transcript/record source and
can support a later message after voice ends or the server restarts.

vCon signing is explicit and verified when configured; signing is not automatic.
Do not advertise unimplemented JWE or protocol-level E2EE. Record validation,
canonical bytes, digest and audience policies apply even to unsigned metadata.
