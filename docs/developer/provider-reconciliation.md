# Owned provider status and reconciliation (G6-037, in progress)

The [local verification report](../evidence/gate-6/provider-reconciliation.json)
continues [message dispatch custody](provider-dispatch.md). State remains schema
31; this adds no durable table or conversion. Qualification is for the recorded
source and isolated macOS arm64 fixtures. G6-037 remains in progress.

Normalized message extensions now require the actual DispatchPermit on status
calls as well as sends. The host mints read-only observation custody: cloning or
forwarding it into creation cannot admit an effect. The owned HTTP transport
rejects mutating methods before opening a connection with this permit. Status
lookups use the retained logical mutation ID and never repeat creation to learn
its result. Existing built-in provider capability declarations remain explicit;
this does not add undeclared provider status operations.

Store admission requires an owned current runtime, an existing eligible effect,
a registered MessageStatus contract, enabled matching route, current original
sender and delegate credentials, current membership and original message audience.
It derives Conversation/all current member Identity read scopes, waits without
SQLite, rederives after acquisition and rechecks the exact current source inside
a read transaction. The permit retains those scopes and the real installation
owner through transport cleanup. It expires at the earlier credential expiry or
35-second observation deadline. Retired runtimes refuse further I/O, while an
already pending transport still holds the kernel lock until cleanup. Administrative
metadata inspection grants no transport authority and cannot mint these permits.

A terminal task, SMS STOP or expired enrollment does not erase an existing remote
effect. Observation admission grants no new send or task continuation. Receipt
settlement still checks current authority and applies the existing separate reply,
SMS, provenance and unassigned terminal-task communication rules. The status permit
must leave the adapter future before settlement takes a writer; actual pending
socket clones retain custody until their cleanup completes. Registration, admission
and receipt settlement in Worker::reconcile run in spawn_blocking.

The real HTTP status fixture starts with one possible effect, holds its status
GET response, and proves an identity revoke returns 503 with unchanged state.
Cancelling the reconciliation future closes the actual pending socket and releases
its authority. A read-only retry records the factual receipt, the original revoke
succeeds, and a further unauthorized lookup performs no network request. The
creation count stays one. Lower-level tests reject POST before any connection,
close a cancelled GET, bound status by source expiry, refuse missing/undeclared
contracts and administrative ownership, and retain a retired runtime's kernel
lock until the final transport permit drops. Contribution conformance also checks
retired status denial and read-only authority refusing creation.

The checkpoint does not qualify voice/SIP network custody, production provider
status capability expansion, bootstrap/provisioning SDK I/O, all resource readers,
post-retirement attributed settlement, every expiry/bulk profile, signed-key Device
policy, retirement watchers, actual HTTP/signed response delivery, the production
canonical listener/full dispatch or transport/SDK parity. G6-020 media and dependent
G6-038 Session work, Linux/remote/browser/live-provider lanes, publication and all
acceptance remain open.
