# Gate 6 membership and audience policy

G6-031 established the store and service policy used by typed events, addressed
messages and records. The [schema-34 membership control continuation](membership-control.md)
adds shared canonical Add/Update/Remove and HTTP/native/SDK/MCP actions. Earlier
policy evidence is historical and does not qualify the current continuation.

`Store::execute_membership` and `Service::execute_membership` accept a
`connect_protocol::membership::MembershipRequest`. Its fields are `operation_id`,
`conversation_id`, `expected_revision` and a nested, tagged `change` object. The
supported actions are add, invite, join, decline, cancel invitation, leave, remove,
update role, share history and revoke history share. `membership_snapshot` and
`membership_invitations` expose bounded current views. Notification wakeups are
hints; committed journal rows and current snapshots remain authoritative.

The service's `execute_membership_authenticated` accepts principals produced by
the local bearer validator, verifies their issuer, installation and expiration,
and delegates to the same current-credential transaction. Reserved task subjects
cannot use it. An ordinary human or AI agent uses an explicitly issued
`Scope::collaboration` credential; no task, model approval or task grant is needed.
The five membership permissions are `conversation.get`,
`conversation.participant.add`, `conversation.participant.update`,
`conversation.participant.remove` and `conversation.history.share`.

## Admission and recovery

Each mutation takes a SQLite immediate transaction, rechecks the live credential,
tenant, operation scope, recipient ceiling and current membership, and compares
the Conversation revision. A tenant administrator can manage membership with
explicit administrator authority but does not thereby become a content reader.
Role labels, including `admin`, never confer a credential permission.

An invitation names one live identity in the same tenant and records its kind,
role, inviter and expiration. The lifetime is positive and at most seven days.
Only its target can join or decline; a currently authorized member or administrator
can cancel it. Leaving/removing its inviter cancels pending invitations. Expired
invitations cannot admit a participant. Readmission retains the Participant ID
and increments its epoch. Participant capacity applies to invited joins and trusted
administrator provisioning. Conversation role updates preserve Session role
snapshots.

Canonical intent includes the expected revision and excludes only the operation
ID. Resource and target sets are sorted, duplicate entries are rejected, and
changed intent under the same owner/operation ID conflicts. An exact retry checks
current authority and projects current membership, invitation or share state; it
never restores a former epoch. Admission, revision, receipt and journal effects
commit together. Infrastructure failure rolls all of them back and permits the
same intent to retry. Expiration during a database lock wait is checked after
the lock is acquired; the credential clock is checked again before commit.

## Content and live presence

An accepted resource records its original participant epochs. Existing all-member
message/file writers freeze that set at insertion. Session sources freeze the
initiator and invited participant; future addressed/multiparty writers supply
their explicit accepted audience. Adding a member never adds it to old content.

History sharing records a bounded set of source references, target Participant
epochs and provenance. The caller must currently read every source, hold each
source's read scope and sharing scope, and be permitted to address every target.
Sharing adds a separately revocable grant; it never rewrites original audiences.
An accepted share persists as a Conversation policy decision until explicitly
revoked. Removing its target immediately makes that target epoch unusable;
readmission does not restore the grant.

`authorize_content` is the common admission hook for message, artifact, Session,
call, SIP leg, context delivery, record, transcript and observation sources. It
requires the current credential, operation scope, recipient ceiling, tenant,
active member epoch and source audience. Unknown and inaccessible references both
return forbidden. The record/transcript/observation hook is policy infrastructure;
it does not implement vCon production or retrieval.

History, inbox, context, task recall, receipt replay, downloads and queued provider
dispatch apply current audiences. Compound journal events must satisfy every
referenced source before pagination and counts. No private-source count is added
to an omitted total. File metadata and bytes use the same decision. Provider
delivery status belongs to its source sender or actual route recipient and still
requires the source audience and current recipient ceiling.

Removal retires only the target epoch's logical Connections and Session presence,
clears readiness and ends its legacy binding observations. Unrelated members and
Sessions remain usable. Session history sharing cannot revive an old media
binding: attachment requires an original, current membership epoch. This is a
durable authority fence, not evidence of a physical provider hangup. Later Session
orchestration applies it to engine cleanup and public events.

## Model exports

Chat and voice selections intersect the caller's permitted sources with the
model participant's current audience and any configured connector credential's
ceiling. Selection precedes bounding. Private owner delivery logs are not model
history. Task exports retain selected original source references but omit the
owner's continuation cursor and private omitted count (`next` and `omitted` are
null in the connector's task projection). The ordinary task recall API retains
its authorized cursor/count; an export is a different projection.

Opaque provider chat memory may be reused only with the same captured Conversation
policy revision, origin/delegate identities, epochs and credential scopes. A late
completion retains its dispatch policy; it cannot relabel old remote memory with
new authorization. Task invocations continue to reconstruct their bounded local
selection without using route memory.

Voice preparation seals the selection before provider attachment. After awaiting
attachment, the server calls `revalidate_vapi_session_context` immediately before
export. This checks current caller/task authority, original Session presence,
each source and the model recipient again. A revoked share or membership cannot
be exported merely because selection happened earlier.

## State and qualification

Fresh state uses schema 31. Prior versions, including schema 26 candidate state,
are refused unchanged. Preserve matching backups/binaries and initialize an
explicit new path as described in [the version decision](../decisions/014-gate-6-contract-version.md).
Backup digests cover invitations, resource audiences, sharing provenance and the
captured provider policy as well as existing private state.

Run `python3 scripts/check-gate6-membership.py` for the scoped qualification and
source/binary/log digests. It uses isolated local fixtures. Generic source-kind
matrix tests qualify the admission hook; they do not qualify native/browser media,
live providers, record generation, canonical transport dispatch, Linux/remote
hosting or full Gate 6 acceptance.

Canonical live readers retain their original member epoch and cannot revive after
removal/readmission; see [canonical subscriptions](canonical-subscriptions.md).
Sender revocation does not rewrite another current recipient's original audience
for immutable retained messages.
