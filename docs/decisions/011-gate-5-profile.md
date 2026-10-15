# 011 · Gate 5 local release profile

Recorded October 13, 2026. The owner directed Gate 5 implementation against the
product and numbered plans. The defaults below make D5-01–D5-05 concrete; their
qualification is required before advertising them as delivered.

Amended October 15, 2026 by the owner-approved impairment-only acceptance
policy `connect-media-acceptance/2`, recorded in
[Decision 012](012-gate-5-impaired-media-proposal.md). The wire/media profile
`connect-video-vp8/1`, strict clean and mute checks, and all existing non-A/V
acceptance checks are unchanged. Fresh matching policy-versioned qualification
is required; this amendment does not promote any retained failed report.

## Media

The required two-party paths are browser/browser, native QUIC/browser and native
QUIC/native QUIC, with duplex audio and VP8 video. Native control is reliable
UCTP; native media uses the existing eight-byte UCTP header and complete RTP in
QUIC datagrams. Browser participation uses authenticated WebRTC. No Vapi vision,
PSTN video, SIP video, screen sharing or multiparty video is implied.

The local profile is `connect-video-vp8/1`:

- VP8, 90,000 Hz clock, 640×360, at most 30 frames/second; 1,000 kbit/s target.
- Native RTP packet maximum 1,180 bytes, including RTP header; no unchecked
  datagram fragmentation. Explicit VP8 repacketization handles larger source
  packets at the transport boundary when required.
- Encoded frame maximum 256 KiB. Incomplete frame assembly expires after 200 ms;
  missing or duplicate packet sequences cannot produce a claimed complete frame.
- Native frame reordering holds at most eight incomplete frames and 64 queued
  fragments across them, with a 200 ms deadline and 256 KiB per-frame bound.
  Different SSRCs/timestamps never share an assembly; completed duplicates do
  not produce another decoded frame.
- Each video packet queue is bounded to 64 packets. Queued media older than
  200 ms is discarded. Audio retains its existing ten 20 ms frame queues.
- The physical peer's shared QUIC DATAGRAM send buffer is explicitly bounded
  to 76,032 bytes (64 × 1,188-byte maximum profile wire packet), independently
  of application queues. This is a byte bound; Quinn's internal evictions and
  queued packet age are not observed. Sampled free-byte/occupancy/pressure
  metadata remains separate from decoded delivery and recovery evidence.
- Keyframe interval at most one second; feedback requests are bounded and
  coalesced. Independent decoder recovery, timestamps and A/V synchronization
  are measured; feedback and periodic keyframes have distinct evidence.
- Clean profile: at least 24 independently decoded moving frames/second for
  60 seconds on each required path. The loss rehearsal retains 3% loss and up to
  100 ms delay variation, plus the existing two deterministic 250 ms burst
  interruptions.
  Fresh decoded motion must recover within two seconds of the burst ending.
  This is fresh-motion recovery, not restoration of full clean quality within
  two seconds. Paired teardown remains within five seconds.
- Generated synchronized audio pulses and visual markers measure receive-side
  A/V alignment. Clean and mute scenarios retain their strict existing sample,
  calibration and skew requirements: median absolute offset at most 80 ms and
  95th percentile at most 120 ms. Clean 60-second windows require at least 42
  uniquely paired markers. Packet receipt and nominal RTP clock rates cannot
  substitute for independently decoded markers. These measurements do not claim
  a native renderer's physical speaker/display latency.
- Only during the unchanged impaired scenario, measured A/V skew and validly
  derived A/V measurement availability are advisory degraded-quality results.
  Exceeding the clean skew limits or having insufficient/ambiguous cadence
  evidence does not alone block impaired acceptance. The raw A/V result and its
  reason remain visible; unavailable evidence never becomes a synchronization
  claim. Malformed observer data, invalid derivation, wrong source identity and
  every existing non-A/V failure still block acceptance. Identified duplex audio,
  the existing impaired moving-video floor of 1 fps, continuous fresh motion,
  two-second interruption recovery, resource/network ceilings, authorization and
  five-second paired teardown remain required. See Decision 012 for the exact
  policy boundary and fresh-evidence requirement.
- During each clean 60-second window, transfer a 1 MiB artifact in 32 KiB
  chunks through the public reliable client interface and independently verify
  its final bytes/hash. Include transfer timing in the media report.
- Sample all owned server, client, codec and browser process trees every
  500 ms. Aggregate peak RSS must not exceed 2 GiB; aggregate CPU must not
  exceed 800% (eight fully occupied cores). On each measured native QUIC hop,
  total received wire rate in each direction, including reliable file/control
  traffic, must not exceed 8 Mbit/s. These are ceilings for this two-party local
  qualification profile, not a production capacity claim. Unobserved queues,
  browser network impairment or playout latency remain explicitly unresolved.

At the codec-neutral video forwarding boundary, a Video MediaFrame retains
complete validated RTP to preserve marker and sequence information. Audio
MediaFrames retain their existing codec-payload convention. This is an explicit
adapter contract, not a new UCTP wire format. The RTP source hop's extensions and
CSRCs are not authority and are not blindly exported to another hop.

The native example uses an explicit FFmpeg codec/capture/decoder dependency,
with generated media for account-free qualification and opt-in human device
capture/rendering. The server does not require FFmpeg and does not decode VP8.
A transport test counts decoded changing frames, not packet receipt alone.

## Installation and control

The initial release target is local macOS arm64. One normal revocable identity
key is the default for agents and apps; task tracking and narrow grants remain
optional. Ending a leg or session preserves the durable conversation and its
source context. Automatic model wake and broader presence scheduling remain
outside this release profile.

Server secrets use explicit Keychain references with an explicitly selected
environment option for development/CI. A missing or locked reference must fail
with an actionable error and cannot trigger plaintext fallback. Native contact
pairing establishes ownership separately from access and SMS consent. Legacy
provisioning secrets must migrate into the selected secret boundary before
release; private plaintext checkpoints cannot be described as Keychain-backed.

## Independent implementation

The independent control client uses Python with pinned aioquic 1.6.1 and public
wire vectors. It implements UCTP/Connect framing and control itself, without the
Rust helper or rvoip UCTP parser. Real QUIC, trust rejection, durable cursor
recovery, uncertain-ACK replay and independently initiated communication are
required. This control proof does not claim a separately implemented media stack.

## Release and evidence

Prepare a checksummed GitHub distribution candidate containing matching server,
SDK/helper, examples, reference client, protocol/connector kit and reproducible
qualification inputs. Final version/tag names are fixed when candidate artifacts
are built. Package registry publication or additional platforms require their own
qualification and explicit distribution decision.

Measure at least three independent fresh-user walkthroughs. The target is a
local message and reply within ten minutes after installation; report installation,
selected host/model authentication and provider prerequisites separately. Failed
attempts are retained. A missed target is not a passing marketing claim.

Use isolated local state and provider fixtures. Historical carrier results retain
their exact revisions; changed provider behavior lacking required live evidence
is a release blocker until a separate bounded rehearsal is authorized. This
profile adds no public ingress, remote deployment or communication-provider
operations to automatic checks. Owner review and publication remain distinct.
