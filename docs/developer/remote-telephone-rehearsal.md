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

Current observations are in
[`remote-telephone-network.json`](../evidence/gate-6/remote-telephone-network.json).
The first Linux attempt rejected an escaped SIP user; a subsequent admission
wait used the wrong nested journal path. Those fixture failures remain retained.
After correction, the actual network run exposes RTCP on the RTP socket without
SDP multiplexing negotiation. The whole rehearsal remains failed. Neither the
telephone component nor G6-054 is qualified by these partial observations.
