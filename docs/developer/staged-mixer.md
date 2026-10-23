# Guarded audio mixer

G6-020 remains in progress. `Orchestrator::enable_staged_participant_control`
connects the general Participant Session controller to the guarded audio ports
registered by a physical native adapter. It requires the exact `MediaPolicy`
instance used by those ports, plus the current `ParticipantPolicy`. Native QUIC
and WebTransport admission uses `admit_peer_session_connection` and the actual
physical peer's existing quota reservation. The raw frame mixer is a separate
engine API; staged admission does not fall back to it.

Preparation selects one active duplex logical audio stream, checks its codec,
and reserves its receiver and controller control permit. Publication stays
closed until the Participant mutation commits under the common scene fence.
The inactive marker refuses new admissions while already admitted physical-port
sends drain before commit. Previously buffered packets cannot acquire the new
admission, and target audio
excludes samples admitted before its current scene epoch. A failed candidate
retires its physical route through the existing supervised cleanup path.

Each 20 ms tick decodes to a mono bus and encodes a separate mix for every
connection. Mix-minus excludes all devices belonging to the receiving
Participant. Codec selection uses the negotiated encoding name independently
of dynamic RTP payload numbers. Each connection owns its codecs and resamplers;
stereo Opus is downmixed and expanded at the boundary. Ingress work, pending
samples, live packet age, and egress queues are bounded. Congestion drops live
audio rather than growing an unbounded backlog.

Source publication and each target's permission to hear that source are checked
with current owned media leases. Mixed queued output retains every contributing
source's original port and private scene generation. Delivery checks all of
them again after dequeue and retains their guards through the bounded local
send. Credential/task expiry and cancellation apply throughout delivery.
`MediaPolicy::current` must be a bounded, nonblocking read and must not reenter
engine state; a tick can already own another lease in the same policy domain.
Queued writers therefore cause a closed admission rather than recursive-reader
deadlock.

Role changes quiesce admitted sends before the Participant mutation commits,
then replace the private scene generation and current speaking permission.
Role labels do not grant speaking permission. A failed or timed-out role drain
restores the old gate. Departure, terminal physical retirement and scene end
retire the private gate explicitly, even when another preparation retains its
owner. A vanished scene never falls back to ordinary forwarding. Managed audio output
also rejects direct ordinary forwarding, so an unrelated port cannot bypass the
scene with an unmixed or pre-admission packet.

A ready replacement port for the same logical audio stream is adopted without
changing Participant membership. Port, credential and scene generation changes
clear pending decoded samples and reset codec history. A changed contributor
set also resets the target encoder and resampler. Unsupported codec layouts are
rejected before adapter opens; incompatible observed payload numbers are
rejected before the old port is quiesced. Attempted physical opens retain their
existing cumulative budget accounting. An additional or different logical audio
stream requires a future explicit subscription/reconfiguration operation.

The local qualification is recorded in
[`../evidence/gate-6/staged-mixer.json`](../evidence/gate-6/staged-mixer.json).
Signed local TLS tests compose native allocation, controlled media policy and
readiness, the trusted participant API, and decoded RTP. They do not install
Connect production policy, public offer/answer/ready or confirmation/presence
workflows. General subscriptions, reliable application data, SIP/WebRTC guarded
admission, browser behavior, the G6-038 durable general Session projection,
remote/provider profiles and Gate 6 acceptance remain outside this slice.
