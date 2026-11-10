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

## G6-058 preparation and activation update

Implementation is integrated and under component verification; this section does
not report a passed transport matrix. The earlier scoped-runner evidence above
remains historical.

The first-connection and replacement paths perform a short authenticated planning step to reserve bounded dormant Core ports and select the original offer's transport/codec plan. They then discard the planning authorization and signed command reader before awaiting SDP, ICE/DTLS, native offer/answer, or adapter resource preparation. The owned preparation token grants no conversation media, data, participant publication, or operation completion. It permits only the original connection offer/answer exchange, bound to the original physical owner, scopes, participant, selected stream IDs and original deadline.

After resource preparation completes once, activation reacquires current authorization for the same original attempt. Ordinary connection publication takes the Store's ordered Write proof and retains it through the controlled Core activation, participant admission and durable original completion. Replacement private admission remains distinct from public ownership handoff; the later public replacement commit acquires its own ordered Write scene. Original credentials, membership, source/target generation, selected plan and deadlines are revalidated. A refused activation retires its candidate; it never repeats negotiation or retires the old owner as success.

Core exposes opaque QuarantinedInitialMedia and PreparedInitialMedia tokens. stage_initial_selected_with_held performs planning without adapter IO; prepare consumes the original resource token and calls adapter preparation once; activate_with_held or activate_muted_with_held consumes the prepared token under fresh authority. Atomic initial mute preserves the existing replacement revision-2 contract without exposing an intermediate unmuted state. Resource cancellation is independent of completed SDP handler cancellation; activation installs the fresh control cancellation while preserving the original resource deadline.

SDK, HTTP and wire request/response schemas are unchanged. An SDP/connection.answer remains resource negotiation, not readiness or successful handoff. Existing audio/keyframe readiness checks run only after authorized private candidate admission; public ownership switch and old-owner retirement remain mandatory. This implementation adds no acceptance relaxation, fallback permission, new timeout or mutation replay.
