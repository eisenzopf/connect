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

`scripts/lib/transport_replacement_target.mjs` now owns the public SDK controller
and Rust target stage. It copies and validates the bounded source, saves the
complete original command and both observed Connection generations before
submission, saves the returned original reference and checks the allocation
root's command, operation, expected revision and record. It starts only the
source-hashed independent executable with private input/output. Completion comes
from the original operation; retirement uses bounded reads of the same source
Connection and a newer generation. The retained Connection keeps its original
generation and the completed destination binding must identify the designated
Device. The caller joins the original source owner through an explicit callback.

The stage restores the same original reference, waits for the original worker,
records forced cleanup as failure, and verifies unchanged executable/config
bytes. It neither decodes packets nor infers completion from negotiation or
traffic. Controlled tests cover copied sources/native capability shape,
bounds/refusal and a mismatched original root that cannot launch a physical
target. Actual repeated transition and full matrix integration evidence remain
separate requirements.

The target stage now polls only its original operation until the journal enters
preparation; acceptance alone does not allocate a candidate. These reads and the
original completion wait share one eight-second budget. The source/retained
generations and original record remain pinned throughout. The backend's metadata
reservation briefly queues writer admission outside SQLite, then resolves every
current scope/fact again before its once-only durable reservation. Packet
freshness, authority and media limits remain unchanged.

Two local captures through the reusable stage independently identify duplex
Opus and original decoded-packet arrival gaps of 16.673 and 39.876 ms. The
twenty-move sequence stops at its original second failure: the first move passes,
the second root stays accepted without preparation/effects, and the target's
early allocation is refused. A separate two-move reproduction records repeated
reservation authority contention. See
[the stage evidence](../evidence/gate-6/physical-native-replacement-stage-local.json).
A newly compiled and independently verified production runtime with the writer
correction completes seven consecutive original moves with independently decoded
duplex audio; its largest decoded-packet arrival gap is 100.783 ms. The eighth
original fails after attach/activation when a read-only readiness observation
returns `Media(Model(Conflict))`. The sequence stops without resubmitting that
original, and all physical peers/server join without force. The journal retains
`native-target-stopped` with no readiness or movement capture.

Readiness capture and observation use nonblocking media-state locks while Core
holds connection fences. The service now defers that conflict until its next
50 ms poll, releases the held invocation and captures a fresh event under fresh
current authority. It keeps the original operation and deadline; every other
error still refuses. This correction requires new production behavior evidence.
Neither these seven observations nor the correction qualify twenty transitions
or the full transport matrix.

`scripts/lib/transport_replacement_audio.py` independently decodes the private
original RTP payloads after native RTCP demultiplexing. It requires exactly 960
decoded samples for each original Opus packet and classifies speaker identity
against the original receive order/timestamps. Its ten-second component requires
at least twenty identified baseline packets and three hundred identified
destination/retained packets. It refuses self audio, an identified old speaker
after the destination, and a decoded packet-arrival gap over one second. This
observation does not measure device presentation, paired AV synchronization or
the full media quality profile. Four controlled cases cover sample/arrival
correspondence, speaker/refusal conditions and that original gap bound.

The source196 readiness correction builds as an independently verified production
runtime. Its new twenty-move sequence stops at original move three: two original
roots and independently decoded audio pass, with a maximum 62.297 ms packet
arrival gap. Caller-monotonic intent-to-first-identified-destination-arrival
observations are 949.656 and 685.529 ms. These are arrival observations, not
device presentation or full-profile transition qualification.

The third original captures attach, activation and actual duplex readiness,
then returns `Media(Model(Conflict))` from Core movement. Its durable commit
effect and operation remain **unknown**. The caller does not resubmit or retry
that original. The new readiness-contention branch was not reached in this run.
A recorded upstream debug-only observer now identifies managed mixer admission,
changed managed state and target-validation refusals before publication. It
changes no commit/retry semantics. Exact production diagnosis remains pending;
a generic conflict must not be treated as permission to repeat an uncertain
movement.
