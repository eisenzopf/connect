# Owned TURN relay rehearsal

G6-054 remains in progress. This fixture supplies loopback implementation
coverage; distinct-network browser/native/telephone, ICE restart, packet loss,
unreachable ports and remote qualification remain separate required evidence.

Use `scripts/rehearse-gate6-turn-relay.py` with a source-bound
`gate6_canonical_webtransport` test executable, coturn 4.18.0 and the existing
synthetic red/blue VP8 files and independent FFmpeg decoder. Prepare dependencies
with `scripts/prepare-dependencies.sh` before compiling. Keep the input manifest
and exact executable SHA with the result; compiling alone is not a fixture pass.

```sh
python3 scripts/rehearse-gate6-turn-relay.py \
  --binary .local/path/to/sealed-gate6_canonical_webtransport \
  --source-manifest .local/path/to/original-build-input-sha256.json \
  --fixtures .local/gate-6/rtc-canonical-video/fixtures \
  --output .local/gate-6/turn-relay/fresh-udp \
  --transport udp
```

Repeat with `--transport tcp` and a fresh output directory. The TCP lane disables
the owned TURN server's UDP client listener. Both lanes request UDP relay
allocations; TURN control over TCP does not mean RTP over TCP between relays.
The fixture does not qualify TURNS, public ingress or a browser profile.

The runner owns one nonroot coturn child bound to loopback, one listener and
sixteen relay ports, at most eight allocations, and a separate eight-port
canonical media range. Peer permissions allow only loopback. No background
service, provider, model account or system trust modification is needed.

The private configuration uses a fresh synthetic shared secret and coturn's
[documented REST credentials](https://github.com/coturn/coturn/wiki/turnserver):
expiry-prefixed username and Base64 HMAC-SHA1 password. Expired and valid
credentials are distinct. Secret values enter only the child environment and
private configuration; canonical server configuration uses explicit secret
references. No secret enters SDP or a canonical intent.

The explicit ignored test first requires expired credentials to produce no
candidate and no connected peer. The valid run then requires exclusively relay
candidates in both offers and answers, actual signed WSS/Store/Core admission,
duplex decoded Opus tones and VP8 pictures, subscription withdrawal/restoration,
video mute/unmute, and original peer/server close with capacity recovery. A
successful summary also requires at least four real allocations and coturn to
exit its allocation drain while no forced server termination is needed.

Tests have a ninety-second outer deadline and coturn has a ten-second allocation
drain observation. Failure retains private logs and joins the owned child;
forced termination is recorded separately and cannot count as an allocation
cleanup pass. The runner reports hashes and counts only. Keep raw logs,
credentials, generated decoder output and synthetic state under ignored `.local/`.
