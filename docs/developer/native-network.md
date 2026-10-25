# Native local and remote network profiles

The assigned G6-048 native profile is qualified in its recorded scopes. The
[source-bound report](../evidence/gate-6/native-remote-profile.json) and
[requirement audit](../evidence/gate-6/native-remote-audit.json) record the actual
checks and operating limits. Named remote and provider rehearsals use the bounded
configuration and designated endpoints recorded by G6-093.

`CanonicalClientConfig` and `CanonicalServerConfig` select `network_profile`.
The default, `"local"`, confines listeners and clients to loopback. The generated
local installation retains its configured localhost certificate and signing keys.
A saved local config can contain an ephemeral port during setup; set the actual
listener address before opening the client. Opening an unassigned destination
port fails before network or authentication I/O.

An explicit remote client configuration includes these fields alongside the
issued Identity/Device credential and signing-key configuration:

```toml
network_profile = "remote"
endpoint = "192.0.2.20:7443"
server_name = "connect.example.org"
trust_certificate = "connect-roots.pem"
timeout_ms = 10000
```

The endpoint is a concrete IPv4 or IPv6 socket address. `server_name` selects the
TLS identity checked against the certificate. Configured trust is mandatory;
there is no verification-disabled or automatic system-trust path. The separately
configured `server_public_key` and its key ID/algorithm/lifetime still verify
signed UCTP replies. Provision both trust domains through trusted setup.

A remote native server explicitly selects its listener profile and TLS assets:

```toml
network_profile = "remote"
bind = "0.0.0.0:7443"
certificate = "connect-chain.pem"
private_key = "connect-tls-key.pem"
```

TLS assets are bounded regular files. The server accepts a DER certificate or a
PEM chain with the leaf first, up to eight certificates and 256 KiB total.
Individual certificates are limited to 64 KiB. The private key is bounded to
16 KiB and supports recognized DER/PEM encodings. Certificate validity is checked
at startup; rustls checks the configured peer identity, chain, validity and TLS
handshake signatures. Client trust accepts an explicit DER certificate or a
bounded PEM root bundle. Install the required intermediate chain on the server.

`http_bind` continues to select a loopback application listener. Hosted HTTPS,
HTTP/3, WSS, allowed origins and proxy policy belong to their explicit hosting
profiles. Native QUIC configuration does not expose plaintext application HTTP.
See the [canonical HTTP reference](canonical-http.md).

`timeout_ms` bounds the complete native connection and authentication exchange,
with a maximum of 60 seconds. An incomplete exchange closes its owned transport.
A successful connection keeps the same signed canonical control dispatch and
bounded native datagram media admission under current server authorization.

`CanonicalClient::rebind_network` binds a new local UDP socket for the same QUIC
peer. QUIC validates the path. The SDK preserves the logical Connection and
operation IDs and checks its current authentication/key lifetime before rebinding.
The new socket must satisfy the selected profile and address family. Rebinding a
network path grants no renewed credential, role, publication permission or media
readiness. The server continues checking current authority on application and
media I/O.

After an actual disconnection, open a fresh authenticated client and retain the
original logical mutation IDs when recovering status or replaying requests.
Previously recorded readiness cannot recreate a physical connection. A configured
Session reconnect grace and explicit operation uncertainty remain authoritative.

Reproduce the isolated checks with the
[versioned runner](../../scripts/rehearse-gate6-native-remote.py) and
[owned Ubuntu guest setup](../../scripts/fixtures/native-remote/README.md). The
qualified graph passes17 exact Rust methods and both guest signing profiles,
including five actual local CLI/bootstrap methods. The guest is a separate local
Ubuntu24.04.5 Linux/amd64 process under x86 emulation; its control/reconnect proof
is separate from the independently decoded NAT media proof. Hosted-server and
performance qualification remain separately assigned.
