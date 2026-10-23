# WebSocket co-located WebRTC audio primitive

G6-024 reuses the optional `rvoip-websocket/media-webrtc` bridge for the
WebSocket fallback's DTLS-SRTP media plane. WebSocket carries control and
reliable data; it does not carry native RTP datagrams. The default stub still
requires the explicit feature. The fixed bridge's default audio profile is
Opus at 48 kHz, PT 111 with the WebRTC SDP two-channel declaration; it does not advertise additional codecs or video. The encoded/decoded qualification payload is mono, matching the frozen Gate 6 audio profile.

A connected but idle caller can begin media later. Creating the media stream
starts one owned remote-track attachment task, rather than relying on a
one-shot wait during ICE/DTLS connection. Track hints and transceiver discovery
use the existing peer API; the existing media stream deduplicates physical
track identities. The task holds weak peer/media-slot references between
bounded polls. It does not confer any host authorization. Both audio queues are explicitly bounded to ten frames, reusing the existing configurable stream constructor internals and 200 ms local send deadline. The regression checks both public queue capacities.

Close cancels attachment, removes the media slot, aborts and joins attachment,
then closes the stream's pumps and physical peer. The shared media-slot lock
prevents an attachment after pump shutdown. Cleanup is serialized and retains
successful completion for repeated callers. A closed bridge cannot recreate its
media stream; dropping a bridge also cancels and aborts its attachment task.

The qualification driver is `scripts/check-gate6-websocket-media.py`; its report
is `docs/evidence/gate-6/websocket-media.json`. The original failing binary,
source-lock, patch, test source and timeout log are preserved privately. The
regression starts no RTP until both connection waits have returned, then sends
real encoded Opus through public `MediaStream::frames_out`, receives and decodes
five frames in each direction, and checks rate, channels, sample count,
non-silence, frequency, timestamps and exact encoded bytes. A separate idle-close
case checks bounded cleanup, receiver retirement and no retained peer owner.

The existing Core WS-to-WS bridge fixture now uses real encoded Opus and decodes
ten received frames. It neither primes tracks nor polls or manually attaches
remote tracks. SDP-envelope, trickle ICE, WSS and library regressions cover the
surrounding optional backend. These isolated engine fixtures use their existing
bearer-stub/legacy signaling helpers and direct SDP where stated. They do not
qualify production canonical Store/journal/service/HTTP/SDK installation or a
live browser. Canonical native media, current authority, DTMF and quality have
separate source-scoped evidence linked by the plan. G6-028 must qualify the
combined reusable upstream contract; Linux, browser and remote/provider
acceptance retain their approved tasks and G6-093 endpoints. Publication is a
separate action. Read the report and plan for actual completion status.
