# Native Session role convergence

The canonical Session role command commits the desired role to the journal and
Session topology. The live native coordinator derives that desired state through
fresh Store-issued media readers. Command completion describes the durable
change; it does not assert that a physical media change completed atomically.

Each live native route checks the current role during its existing lifecycle
poll. A changed role acquires a bounded private projection of all that
Participant's current native Connections. A busy policy read or coordinator
defers the attempt. Every reader is released before the next poll. The original
role command and its operator credential are not retained or replayed.

The held participant policy checks the actual original principal, current
membership epoch, Conversation, Session, topology revision, desired role,
private mutation ID, native resource activation, cancellation and deadline.
Callbacks reuse those readers without reacquiring SQL or grants. Local role
preparation and commit have a 250ms bound. A later poll checks actual coordinator
state before attempting another mutation. Returning to an earlier role gets a
distinct ID from the new durable topology revision.

Canonical role labels remain `ParticipantRole::Custom` values. They do not grant
permissions or acquire the semantics of Core's built-in Observer role. Audio
permission is aggregated across the Participant's current Connections, so a
data-only endpoint cannot disable its authorized audio endpoint. Individual
packet and subscription checks still enforce each Connection's current grant.
The process diagnostic `role_projections` counts only successful changes whose
actual coordinator role matches the current desired role. It confers no authority.

Native offer and stream effects use the durable operation record ID for their
provider-wide correlation key. Logical operation IDs remain owner-scoped and
unchanged in wire requests, receipts and history. Independent owners can use the
same logical ID without colliding in the effect ledger.

The production runtime regression changes a Participant with audio and data
Connections through three roles, including returning to the first label. It
checks actual coordinator convergence and real audio/JSON data after every
change. A separate controlled Store regression begins offer and stream effects
for two owners using identical logical IDs; its readiness rows are fixtures.

Exact-stream audio/video/screen/data routing is installed. The current
[general Session runtime evidence](../evidence/gate-6/general-session-runtime.json)
records the local G6-038 completion scope.
Participant video, screen and data selectors now use the production
[native reconciliation policy](native-subscriptions.md), with fresh authority,
server reoffers and durable captured outcome custody. Their three production
regressions are enabled. Later publication, source retirement and recording
recovery have additional production camera cases.
This document does not qualify Linux, browser, provider or remote profiles or
close Gate 6 acceptance.
