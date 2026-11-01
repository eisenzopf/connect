# Physical native replacement peer

The independent Rust SDK consumer now has a `replacement_peer` mode. It uses
only `connect_client::canonical::CanonicalClient` and the public allocation,
media-start, signed-answer and native-datagram interfaces. The SDK controller's
movement handle remains a metadata handle; it does not own the destination's
physical media connection.

The parent saves the complete original command and operation root before target
IO. Its private input includes a CLI-issued native configuration path, one
sendrecv Opus stream, explicit native codec capabilities, 20 ms encoded mono
48 kHz packets, an SSRC and a new private observation path. Native offer
`audio_codecs` entries contain codec parameters; Session capability name arrays
are a different interface. Source input is bounded to 30 seconds and the frozen
1180-byte complete-RTP limit. Negotiation is bounded by eight seconds and 128
verified hints. Unsolicited hints never establish original-operation completion.

Both negotiation legs must retain the original resource/operation scope and
selected codec, hop handle and stream ID. Frames keep their original source
schedule; sources delayed by 200 ms are discarded rather than relabelled.
Received native datagrams remain private and bounded. They include RTCP clock
compounds as well as RTP: classify them with the public native observer before
decoding Opus. Clock compounds are not audio frames or media-readiness evidence.
The consumer joins its original client on success and failure and records the
work phase before shutdown can retire a failed allocation.

One local macOS arm64/EdDSA rehearsal passes with the verified production server,
three SDK helpers and an independent physical target. The original move and
captured destination binding complete, the original Connection is observed
retired, its incoming path becomes quiet and its owner explicitly joins.
Independent Opus decode identifies the original 440 Hz speaker before the move
and destination 660 Hz speaker afterward at the retained peer; both old and new
receivers identify the retained 880 Hz speaker. The measured decoded-packet
arrival gap is 40.033 ms. A fresh SDK process subsequently restores the same
original completed reference without another mutation.

This is one audio-only component observation. It does not establish device
presentation timing, all privacy/quality conditions, twenty transitions,
video/paired AV alignment or a complete transport cell. Earlier diagnostic
failures and the corrected RTCP/audio demultiplexing audit remain recorded in
the [local evidence](../evidence/gate-6/physical-native-opus-replacement-local.json).
Integration of this target and its independent decoder into every required
matrix producer remains open.
