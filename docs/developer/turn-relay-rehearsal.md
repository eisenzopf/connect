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

Tests have a ninety-second outer deadline. Coturn must explicitly report zero
allocations within ten seconds of its drain signal, followed by an independently
bounded ten-second join of that original process. Failure retains private logs and joins the owned child;
forced termination is recorded separately and cannot count as an allocation
cleanup pass. The runner reports hashes and counts only. Keep raw logs,
credentials, generated decoder output and synthetic state under ignored `.local/`.


For the explicit shared-endpoint All-policy lane, add `--ice-policy all` to the
UDP command and use a fresh output directory. It publishes host and relay
candidates while STUN and TURN share the same local/server UDP tuple. Expired
credentials are still tested with relay-only ICE before the valid All-policy
scene. This lane checks decoded duplex and allocation drain; it does not assert
that All selected a relay over its available direct candidate or establish
remote server-reflexive reachability. The protocol regression independently
checks the Binding transaction and reflexive candidate without early TURN
completion, including an unrelated transaction that must not produce readiness.


The first All-policy run passed decoded AV and logged four lifetime-zero
refreshes/closed allocations, but failed the former combined ten-second
allocation-drain/process-exit observation. That failure remains in evidence.
The [coturn4.18 listener loop](https://github.com/coturn/coturn/blob/4.18.0/src/apps/relay/netengine.c#L1294)
uses five-second event dispatches and dispatches again after announcing drain
completion. Observation version2 keeps the ten-second zero-allocation limit
and separately bounds joining the original process by ten seconds after that
report. A forced termination or absent zero report still fails the rehearsal.
This fixture timing change does not qualify a media latency or remote profile.
