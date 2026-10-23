# Durable Session outcomes

G6-046 records the original terminal outcome independently of Core's disposable
Session containers. Schema 76 introduced freezing of one stable version-8 document UUID and
metadata intent in the same SQLite transaction that first changes a Session to
`ended`, `cancelled`, `rejected` or `failed`. An initial terminal insertion also
creates an intent. Rollback removes both changes; repeated terminal observations
do not create another document or rewrite the original facts. Historical schema
files remain unchanged; opening an older schema still requires the explicit
backup/reinitialization workflow.

The named installation policy `connect-session-metadata-v1` captures the outcome,
requested media, original timestamps, bounded reason and actual reason code when
available, Participant appearances and durable Connection lineage. It does not
capture audio, transcripts, provider identifiers, endpoint addresses, decoded
media or consultation context. A ready record explicitly reports
`capture_complete:false`, `recorded_audio:false`, disabled raw capture and an
unavailable transcript. A Session that never negotiated media still gets an
outcome. Metadata records do not manufacture historical physical readiness.

The frozen intent contains at most 256 appearances, 512 Connections and 256
original Session audience entries. Counts preserve explicit omissions rather
than claiming complete capture. In the schema-77 read format, whole-record
capture diagnostics have their own source, separate from the shared outcome. Appearance/Connection states and generations
are labelled as facts at the terminal intent, before later cleanup changes them.
The noncritical `connect-session-metadata` extension declares the Party fields
`connect_participant_id`, `membership_epoch`, `role` and
`state_at_terminal_intent`. Local Conversation/Session linkage stays in the
index; the vCon group field is not used. Baseline creation has a reserved storage slot after ordinary records
reach their per-Session capacity.

Document, outcome and appearance rules select the Session source's original
Participant/epoch audience, not all current Conversation members. The separately
named `connect-connection-owner-metadata-v1` policy selects only each Connection's
original owner epoch for its lineage metadata. This source selection does not
grant access to media, another Connection or a private consultation's content.
Shared metadata rules use Core's policy audience and the original epochs commit
once in `session_record_audiences`, keeping the complete 256-Participant profile
within the one-MiB source-manifest bound. The policy audience requires the
retained Store source grant; it is not a blanket read grant. Future read projection
must check current authorization and the matching epoch under the same retained
grant. A manifest Participant ID alone cannot authorize a rejoined membership.

`Service::finalize_record_outcomes` processes at most 32 intents per call. It
constructs deterministic canonical records and complete source manifests from
the frozen original facts. Per-record custody serializes finalizers. Record
bytes, source epochs and readiness commit atomically, releasing the temporary
snapshot in that transaction. A failed transform rolls everything back. A lost
response is resolved from the durable state; a completed intent never runs the
transform again. Redaction/deletion cannot be undone by replaying an old capture
snapshot because that snapshot was released when storage committed.

The canonical server owns a supervised `session-records` worker. It runs one
bounded batch per second, discovers pending work after restart, and stops before
installation retirement. One failed intent does not prevent the other items in
its batch from progressing. Unconfirmed storage is visible as degraded local
worker readiness with a safe reason code; neither cancellation nor an error
claims the write had no effect. Worker readiness describes the worker, not
completion of every record. Standalone hosts can explicitly own the same Service
finalizer; no detached task is created by Store or Service constructors.

The [native record reader](native-record-retrieval.md) separately composes current
source authority and protected record delivery. Readiness discovery, HTTP
response custody and expiring references have component evidence linked from
the Gate 6 plan. Named retention now uses the frozen terminal policy and owned
worker described in [record-retention.md](record-retention.md). Remaining source
profiles and full auxiliary qualification are still G6-046 work. The
outcome-production runtime case uses privileged Store inspection; native reader
qualification must be assessed separately.
