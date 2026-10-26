# Canonical WebRTC signaling component

G6-051 is in progress. The internal TypeScript module
`browser_webrtc_signaling.ts` implements a signaling owner against the frozen
`connect/2` envelope shapes. It is not exported from the browser entry point yet.
The canonical server still refuses this substrate; the older browser voice/video
entry points still use their historical signaling dialect. This component does
not establish browser fallback support or media interoperability.

The component borrows the authenticated `BrowserClient` and takes ownership of
one supplied `RTCPeerConnection`. The caller configures transceivers, channels
and capture before transferring that peer; capture tracks remain caller-owned.
Conversation, Session, Connection, participant, revision and logical operation
IDs are explicit. The component snapshots offered capabilities and streams before
its first asynchronous operation. It gathers complete ICE before sending one
signed `connection.offer` with `substrate: websocket+webrtc` and the frozen
`substrate_setup` SDP shape.

The original exact SDP, payload, scope and submission-attempt flag remain
available after failure. A missing answer, lost response or timeout cannot
generate another SDP offer or operation ID. Recovery must inspect the original
operation using current authorization. Local close/cancellation does not issue
durable operation cancellation or invent a remote outcome. The shared client's
existing timeout policy can retire uncertain physical control; this helper does
not add a second control transport or retry policy.

An answer must match the original Conversation, Session, Connection and operation,
name the same substrate, and select unique offered stream IDs and kinds. SDP is
bounded to 512 KiB and applied through the browser API. Applying an answer is a
signaling observation: actual media requires independent ICE/DTLS, decoded
content and presentation evidence.

Canonical incoming ICE uses `sdp_mid` and `sdp_m_line_index`, converted explicitly
to the browser's camel-case fields. Candidates arriving before the answer wait
until the remote description is applied. One serialized queue has a lifetime
limit of 64 candidates/64 KiB, including work held inside the browser API; one
candidate is limited to 4 KiB. Foreign scope and operation candidates are ignored.
Malformed candidates or exhausted bounds retire the owned peer. Terminal
Connection/Session/Conversation events and shared control loss remove listeners,
gathering callbacks and queued candidates, and close the peer once.

Verification uses controlled peer and WebSocket boundaries. Two signing
algorithms exercise the actual `BrowserClient`, signature verification and
canonical frame encoding, without a WebTransport API. These are component tests,
not TLS/network, installed Chrome/Firefox, Core WebRTC or actual media tests.
The SDK regression suite has 276 passing tests at this checkpoint.

Next, canonical server dispatch must bind the WebRTC adapter to current service
authority and durable operation/physical lifetimes. Then integrate capture,
track/data routing and the public browser surface, remove the old offer-ready
dialect, and run forced WT/UDP failure in Chrome and Firefox with the same
Conversation and transition intents. Current authorization, uncertainty,
permission/background/resume, loss and owned teardown remain required.
