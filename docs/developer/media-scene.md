# Owned general media scene (Gate 6 in progress)

The staged participant scene owns each admitted controller's input queues. Audio
is optional. Video, screen and ordered data use the guarded direct-subscription
program; audio uses the guarded mixer and its retained subscription intent.
Participant admission uses the existing physical connection and peer quota.
A non-audio connection does not require audio receive permission.

Preparation reserves stream control, verifies the exact conversation, Session
and policy domain, installs inactive ownership markers, and takes unclaimed
inputs. Admission drains sends on every original port before activation. Media
captured before scene activation cannot enter a newly admitted receiver. A
managed controller never falls back to unmanaged forwarding after retirement.
Publication, queued delivery and local IO retain and recheck the original scene
owner and generation alongside current media authorization and transport guards.

The scene adopts replacement logical-stream inputs after successful stream
control. Direct delivery visits only registered scene controllers and actual
explicit selections. Video remains bounded and expires. Ordered data retains
one pending packet per source input and retries unfinished listeners after
backpressure or temporary authorization-lock contention. Completed listeners
are removed from that packet's pending work, preventing duplicate retry delivery.
There is no idle policy reader or retained authorization grant. Selection changes,
scene departure, transport replacement and expiry still fence queued work.

A stable audio consumer exists even when no audio is selected. At most one audio
stream is selected per controller. Adding, ending or replacing it uses the same
consumer ticket as participant role changes, drains admitted audio IO, and
preserves the receiver's subscription program. The mixer adopts the new stream
and drops old codec state and queued contributions. One-way audio ports require
matching publication and receive directions; a reversed subscription is refused.

The source-bound runner is `scripts/check-gate6-media-scene.py`. Its final report
records source digests, private logs, executable hashes and required behavior
cases. Draft code and exploratory logs do not qualify a checkpoint.

Qualification is local macOS arm64. Signed QUIC and WebTransport fixtures use
real RTP with controlled readiness and policies. They do not prove video decode
or rendering, browser media, production Store policy or public negotiation
handlers. Core ordered-data backpressure is not native reliable-data transport.
Non-audio Participant selectors, audio selectors for owners without selected
audio, the approved reliable message transport, durable subscription operations,
public offer/answer/ready and confirmation/presence installation, other guarded
transports, general durable Session projection and the remaining migration are
still open. G6-020 and Gate 6 acceptance remain in progress.
