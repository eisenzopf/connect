# Typed original private SIP source

The original quarantined SIP candidate now has a `private-initial-sip` snapshot.
It names both the private child and its original consultation, canonical
Connection, Core connection, SIP Call-ID, generation, Participant epoch and
Endpoint/Device/route credentials. Public `initial-sip` receipts cannot describe
a private candidate. Strict deserialization rejects private fields smuggled into
a public snapshot; recorded presence, departure and captured stream revisions
also check the actual receipt kind.

Fresh physical metadata requires the original SIP reservation, own private
consent, completed child and confirmed call effect, exact committed private
connected observation, current generation/runtime, current own Endpoint/Device,
own credentials and original Session presence. Captured stream revisions can
advance current readiness while preserving the original revision-two quarantine
observation. Permission withdrawal does not fabricate physical call absence.

The Store remembers private origins only after private readiness commits, in a
separate bounded runtime-local registry. A failed readiness transaction cannot
seed it. Captured cleanup removes the original entry. Snapshot resolution needs
that actual origin and compares every field with fresh facts; a database backup
or restarted runtime cannot reconstruct an origin or lend an old runtime's call.
The registry retains no authority reader and never enters public Session
readiness, activation or reconnect coverage.

Physical metadata deliberately does not borrow the original controller's command
credential, deadline or lease. This prevents circular validation when a future
established private lifetime checks its captured SIP binding. Metadata itself
returns no media or signal read. Selection still needs current original execution
authority; continuing private IO still needs its own exact sealed private
lifetime. Existing public media/signal APIs refuse this private source, and the
bounded admission reader refuses ended execution authority.

The Store consultation planner additionally supports an exact native-primary/SIP
candidate pair. Its selected private IDs must equal the original registered
primary and candidate. A SIP candidate must carry this consultation's distinct
private snapshot; a native candidate still requires its native registration and
an ordinary SIP primary. The candidate's private floor must name the original
primary, and an existing primary floor cannot belong to another pair. Both sides
need current subscription permission and reciprocal recipient ceilings.

The private SIP selection reader is constructed only inside that complete
planning interval, with fresh original consultation authority and physical facts.
It is distinct from the bounded admission reader and cannot seed public readiness.
Both directional edges name only the exact private pair; the retained remote and
the candidate Participant's public native sibling receive neither edge. A cloned
SIP reader keeps the complete interval alive until it is dropped. Planning creates
no root effect, native admission, call or audience change.

The [recorded private SIP lifetime](private-sip-candidate-lifetime.md) now extends
Store attempt/completion and sealed physical metadata validation to the original
SIP candidate at schema 108. This is Store planning/recording for G6-046. The
runtime coordinator still selects native candidates. Exact return, pending and
established packet intervals, actual private audio and INFO/RFC4733 remain implementation and
qualification work. The controlled
Store cases verify the source boundary; the existing actual local SIP/Core
rehearsal checks admission/quarantine/cleanup regressions. Neither qualifies a
private audience switch or completes a Gate 6 clause/task. There is no new HTTP
endpoint; clients continue using the existing consultation and operation methods
in the [server HTTP reference](canonical-http.md).
