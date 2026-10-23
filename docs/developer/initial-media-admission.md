# Initial media admission

A retained stream controller may be constructed for a current verified Connection
in Preparing or Ready state. It can stage its first candidate under the exact
member, presence, endpoint, device, Connection ID and generation. This does not
activate a stream or authorize packets. Commit still requires an Active
Connection, exact current authority and matching source/sink readiness. After a
successful commit, a regression to Preparing or Ready cannot start another change.

`MediaAuthorityLease::control_owner` lets a policy rederive mutable relationship
and readiness facts under its original owned control grant. The implementation
must use that same grant without reacquiring its lock or replacing credentials.
The default returns the original snapshot and cannot advance a preparing
Connection. Core checks cancellation and the original deadline before and after
this callback, then checks the frozen principal and relationship scope. A held
control lease is retained throughout the operation. Packet admission, forwarding
and dequeued sends continue to acquire ordinary fresh current policy.

Failure, cancellation and stale readiness roll back provisional ports. They retain
cumulative binding accounting and burned transport handles; retry uses the same
controller and cannot revive an old port. Unrelated active audio is preserved.

The scoped runner is `scripts/check-gate6-initial-media-admission.py`. Its report
records exact source, log and binary digests. Core cases exercise Preparing,
Ready, Active and regressed state, stale generations, endpoint revocation,
callback cancellation and a real queued grant writer. Local authenticated TLS
QUIC and WebTransport cases reject RTP before commit, refuse Ready-only commit,
retry on the same controller with preserved quota/history, and decode RTP after
activation. Their readiness and current-policy projection are controlled fixtures.
A signed ACK reports the engine revision only.

G6-020 remains in progress. This change does not implement canonical
connection.offer/answer/ready, real endpoint confirmation, Participant presence,
production Connect media-policy installation, retained credential rotation,
managed mixer adoption, reliable data, or WSS/WebRTC/SIP media qualification.
No Gate 6 acceptance criterion or remote/provider/browser profile is closed.
