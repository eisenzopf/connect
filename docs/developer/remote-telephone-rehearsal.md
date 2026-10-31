# Owned distinct-network telephone rehearsal

`scripts/rehearse-gate6-remote-telephone.py` runs a fresh Linux arm64 synthetic
SIP/RTP peer in the pinned local Lima container against a Mac canonical server
using the explicit Remote network profile. A public native SDK audio peer in the
same canonical Session provides the opposite tone. No provider, PSTN, SMS, model
account or physical telephone is used.

The operator supplies one concrete owned Mac IPv4 interface. The runner reserves
one signaling port and sixteen media ports on it before launching the server.
The canonical telephone configuration uses that interface for media bind,
advertised signaling and public RTP, retaining the media port numbers. Native
control remains on the authenticated local fixture listener with normal signing
and certificate trust.

The Linux peer is behind an explicit owned UDP NAT witness. Two independent
sockets establish nonce-bound guest mappings to the Mac through the existing
`192.168.5.2` Lima gateway. The witness accepts only the exact guest tuples and
fixed server signaling/media tuples. SIP and RTP bytes cross the VM network in
both directions; the witness records forwarding and foreign-packet counters.
The peer's advertised RTP port and its following RTCP port are both owned. This
is a controlled peer-side NAT translation, not direct WAN or unsolicited inbound
reachability qualification.

Before forwarding the first INVITE to Linux, the witness opens the actual
journal read-only and requires the original initial-call effect to be submitting,
with the same call ID, attempt and correlation. The Linux peer validates the
synthetic destination, offered PCMU payload, explicit media IP and bounded port
range. It accepts RTCP multiplexing only if offered and answers that option
explicitly. Compound RTCP is structurally checked separately from PCMU RTP;
unnegotiated RTCP on the RTP socket fails the rehearsal.

The intended positive run uses the original signed public Session invite and
replays it without another dialog. The public native audio peer explicitly
selects the telephone stream, then both sides send different four-second PCMU
tones at 20 ms cadence. Independent FFmpeg decoding must find the opposite tone
and exclude the sender's own tone. The original signed Session end must reach
the native peer and produce a SIP BYE, with one durable call, one durable native
offer, released physical peers, the original SIP listener released, joined NAT
worker and original guest/server process exits. Forced cleanup never passes.

Prepare dependencies and build the source-bound `gate6_canonical_runtime` test
executable with `test-support`, then run with an actual owned interface:

```sh
python3 scripts/rehearse-gate6-remote-telephone.py \
  --server-build PATH_TO_BUILD_SUMMARY \
  --server-inputs PATH_TO_BUILD_SOURCE_HASHES \
  --host-media-ip OWNED_MAC_IPV4 \
  --output .local/gate-6/telephone-remote/FRESH_RUN
```

The pinned image is
`sha256:533cdd9c6983863f0c7ae50b3a4f9a6157bfe5b88f84b06dd44a5c61e3234bd0`.
This lane uses its Python runtime; it does not start Chromium or PulseAudio.
Private nonce/configuration, databases, raw logs and captured synthetic media
remain under ignored `.local/`. The runner hashes actual build inputs, fixture
sources, image and executable, and checks them again after execution.

Recorded observations are in
[`remote-telephone-network.json`](../evidence/gate-6/remote-telephone-network.json).
The first Linux attempt rejected an escaped SIP user; a subsequent admission
wait used the wrong nested journal path. Those fixture failures remain retained.
The original network failure exposes RTCP on RTP without SDP multiplexing
negotiation. That failure remains retained. After the negotiated routing fix,
both EdDSA and ES256 pass the complete direct and packet-loss profiles: opposite
decoded tones, original replay, signed Session end/SIP BYE and original drains.

The recorded routing fix is in
[`negotiated-rtcp-routing.json`](../evidence/gate-6/negotiated-rtcp-routing.json).
It retains adjacent original sockets and carries actual SDP mux/split RTCP
negotiation into the media owner. All 52 UDP/allocator and seven SIP unit
functions pass in the initial checkpoint. The final manual SR/RR/RTCP-BYE/close
destination fix passes all 73 session, UDP and allocator functions. The four
initial network profiles remain retained. Final production runtime266 and all
four EdDSA/ES256 direct/loss267/268 profiles also pass on the final source,
with five separate-port RTCP packets per profile including RTCP goodbye. Four
SIP integration regressions and four configuration functions pass on that source.

Use `--network-profile loss` for deterministic loss. The owned NAT witness drops
every fifth PCMU RTP packet independently in each direction, records seen and
dropped counters, and leaves SIP and RTCP untouched. Decoder thresholds and
original-owner drain requirements are identical to the direct profile. This is
a functional controlled-loss fixture, not a latency or load qualification.


## Native Linux x86_64 host and peer

The `telephone-network` lane in `gate6-linux-fixtures.yml` builds the current
server fixture on native Ubuntu 24.04 x86_64 and runs the original four
EdDSA/ES256 direct and 20% PCMU RTP-loss cases. The synthetic Python SIP/RTP
peer runs in the pinned Linux/amd64 image as the host operator UID, with a
separate namespace on one fresh internal Docker bridge. The owner verifies
that bridge's identity, name, label and concrete non-loopback gateway before
using it as the SIP/RTP host interface. The peer accepts that gateway only in
the explicit owned-bridge mode, with the same advertised host address.

The existing nonce-observed fixed NAT, independent opposite-tone decoding,
journal-before-INVITE, original-INVITE recovery, signed Session end and original
socket/peer/server-drain checks remain required. The owner retains its exact
network identity in ignored private state before creation, recovers only that
name/label after an uncertain creation reply, and removes the bridge only when
it is still the exact owned empty network. Failure to confirm cleanup is a
failed component. Credentials, addresses, nonces, PCM and raw logs remain under
ignored `.local/`; the CI artifact includes only bounded facts and hashes.

```sh
python3 scripts/check-gate6-linux-hosting.py --telephone-network
```

This covers the bounded synthetic telephone addressing component. Provider,
PSTN, WAN, native video, complete transport matrix and performance qualification
remain separate approved plan work. A Linux result must be recorded separately
from the earlier Linux arm64 peer/macOS host evidence above.
