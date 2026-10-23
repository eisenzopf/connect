# Attachment access and recipient receipts

Attachment audiences and message audiences are separate frozen sets of participant
IDs and membership epochs. Upload with an explicit `to` (`"all"` or participant
IDs); `all` resolves once at acceptance. Finishing or retrying an upload does not
add later members. Sending a reference requires ready, unexpired bytes, matching
ID/content type/size, and current access for every selected message recipient.
A history share of a message alone does not share its files. Revoking a file share
hides its reference as well as its bytes. Context remains bounded, URL-free metadata.

`artifact_inspect` returns resumable progress under `artifact_get` authority.
`artifact_upload_link` requires the original owner and `artifact_append`;
`artifact_download_link` requires `artifact_get` and an exact message/file binding.
Issuance accepts `expires_in` from 1 to 300 seconds. Expiry is also capped by the
credential, artifact, task and invocation grant. Returned `ArtifactLink.path` is a
relative path under the configured server origin; never resolve it against a
caller-supplied Host or arbitrary external origin. Canonical MessageSend still
accepts ID/type/size references and rejects caller URLs; canonical wire rendering
belongs to G6-037. Full bytes remain available through these authorized operations.

Links are installation-, runtime-, credential-, participant-epoch-, purpose- and
resource-bound HMAC references, not bearer credentials. Every use rechecks current
message/file access, state, integrity metadata, expiry and recipient ceilings.
Copied links fail for another credential even if that identity can read the file.
Restart and backup restore invalidate links; stable artifact IDs and chunk mutation
receipts survive. While an upload remains in progress, reissue a link and retry the same request ID,
offset and bytes. Inspect current metadata to reconcile an already finished upload.
The transient link is excluded from the chunk's logical intent. Changed bytes still
conflict; a retry cannot append twice. Deletion, cancellation and revoked grants
prevent further link use.

All SDKs expose `artifactUploadLink`/`artifact_upload_link`,
`artifactDownloadLink`/`artifact_download_link`, `artifactLinkAppend` and
`artifactLinkGet` (snake case in Python/Rust). The generic JSON operations have the
same names in snake case. Ordinary HTTP credentials can PUT/GET bounded raw bytes
at the returned path, with `request_id`/`offset` or `offset`/`limit` query parameters.
Downloads use `application/octet-stream`, `Cache-Control: no-store` and `nosniff`.
Delegated clients redeem links through Task Invoke, preserving the task/grant
context. A delegated URL cannot be redeemed with a borrowed parent credential.

`message_receipts` pages current recipient-specific projections for one original
message. The sender sees permitted recipients; a recipient sees only its own
facts. Unaddressed members and history-only shares cannot inspect or report old
receipt authority. Current membership epochs, identity status, credential/task
recipient ceilings and preferences apply before paging. Cursors are authenticated
and bound to the reader and message. There are no hidden-recipient totals.
Cursors page the current projection; refresh from an absent cursor to check changed
status. Receipt status pagination is not a change feed.

Native and provider channels have separate delivery IDs and durable facts:

| State | Evidence |
| --- | --- |
| queued | Durable native journal or provider intent; no client/provider delivery claimed |
| unknown | Dispatched or uncertain provider attempt; no permission to send again |
| accepted | Provider accepted the effect; an answer's existence alone remains acceptance |
| failed | Failed, cancelled, suppressed or unsupported provider attempt |
| delivered | Verified provider delivery or an explicitly attributed native client report |
| read | Explicit report by the authenticated original native recipient, with read opt-in |

Projections retain stronger evidence across duplicates and reordered callbacks:
read, delivered and failed cannot be downgraded to acceptance, uncertainty or queued
state. Delivery evidence wins over a later failure. A provider's completed answer
is an ordinary separately correlated message, never an inferred read receipt or
human action. Carrier callbacks cannot emit native read facts. Client reports use
`provenance: client_reported`; this is not proof that a person read anything.

`receipt_preferences_get` and `receipt_preferences_set` inspect/change the caller's
current membership preferences. Delivery reporting/exposure defaults to true;
read defaults to false. `message_receipt` accepts only `delivered` or `read`, supplies
no selectable actor, and requires original current recipient authority. A suppressed
read can record delivery when delivery is enabled; with both disabled it records
neither. Optional read facts are never backfilled by changing a preference. Current
preferences also hide retained receipt events and status before paging. Changing
preferences cannot remove internal provider delivery evidence or revive SMS work.

Native reports create separate `message_receipt` inbox events only for the original
sender and reporting recipient. They do not create messages, responses or task
replies. Their source message ACL, current subject epoch, reader ceiling and current
preferences are rechecked on replay. Provider facts are retained with the existing
protected adapter observations/job state; querying receipts does not dispatch work.
Task and grant holders can query receipts with delegated history authority. Receipt
reporting and preference changes are ordinary recipient actions and are not granted
implicitly to a task invocation. SMS consent/STOP suppression remains authoritative.

Fresh state uses schema 31. Prior candidates, including schema 26, are refused
unchanged; back up the original database and explicitly provision a new state path.
No automatic migration, reset or downgrade is provided. Backups retain receipt facts,
preferences, frozen audiences and stable identities; restored links must be reissued.
The HTTP contract is [openapi-attachments.json](../../contracts/openapi-attachments.json).
Qualification is recorded only by `scripts/check-gate6-attachments.py`; focused local
tests alone do not qualify G6-034 or Gate 6 acceptance.

Canonical `message.read` uses the same original recipient, opt-in and logical
receipt ledger. The [canonical receipt continuation](canonical-receipts.md)
projects typed hints with explicit provenance before their cursor-bearing events.
Wire timestamps are diagnostic; shared receipt evidence retains server observation
time and never establishes human action.
