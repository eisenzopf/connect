# Explicit remote media configuration

G6-054 is in progress. These configuration fragments document the addressing
contract; they are not a qualified deployment or a complete server configuration.
Use the existing TLS, signed control, provisioning and current authorization
configuration alongside them. Example addresses are documentation placeholders.

```toml
network_profile = "remote"

[webrtc]
profile = "opus-vp8"
max_peers = 16

[webrtc.addressing]
bind_ip = "10.0.0.8"
port_start = 50000
port_end = 50063
public_ip = "192.0.2.10"
ice_policy = "all"

[[webrtc.addressing.ice_services]]
urls = ["turn:relay.example.invalid:3478?transport=udp", "turn:relay.example.invalid:3478?transport=tcp"]
username_ref = "RTC_TURN_USER"
credential_ref = "RTC_TURN_PASSWORD"

[telephone]
bind = "10.0.0.8:5060"
media_bind_ip = "10.0.0.8"
advertised = "192.0.2.10:5060"
media_public_ip = "192.0.2.10"
media_port_start = 51000
media_port_end = 51063
routes = []
initial = []
```

`public_ip` and `media_public_ip` require port-preserving one-to-one NAT. Each
allocated media port keeps its number. `advertised` is the SIP signaling contact,
not the RTP address. Wildcard SIP binds additionally require concrete media and
advertised addresses. Verified Store endpoints and administrator-selected route
policies still decide telephone destinations; empty route arrays grant none.

Protect TURN values through the server secret reference file selected by
`CONNECT_SECRET_REFERENCES`, for example:

```toml
version = 1

[references.RTC_TURN_USER]
storage = "environment"
variable = "RTC_TURN_USER"

[references.RTC_TURN_PASSWORD]
storage = "environment"
variable = "RTC_TURN_PASSWORD"
```

Keep this file and all values under protected private state. Values are read
when the canonical host is constructed; this component does not implement live
TURN credential refresh. Browser ICE credentials are separately scoped values
provided by the trusted application backend. Do not send a TURN REST shared
secret to a browser or place it in a canonical intent.

`ice_policy: "relay"` requires an explicit TURN service. The browser wrapper's
`iceTransportPolicy: "relay"` independently requires TURN and verifies that the
native peer retained that policy before an offer. Local profiles permit only
loopback addressing and ICE services. No public STUN server is implicit.

The current recorded driver patch implements TURN control over UDP/TCP with UDP
relay allocations. Secure STUN/TURN schemes refuse in that driver; they remain
unimplemented. Direct, relay, expiry, restart, loss, blocked ports and allocation
cleanup need their own execution evidence. An HTTPS health response proves
signaling reachability; audio and video still require their actual readiness and
decoded media observations. See the [owned fixture](turn-relay-rehearsal.md) and
[current evidence](../evidence/gate-6/remote-media-addressing.json).
