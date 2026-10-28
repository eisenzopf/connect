# Signed canonical native runtime

`connect serve --config .local/signed/server.toml` runs the production
canonical QUIC listener. [Fresh initialization](canonical-bootstrap.md) now writes
its protected signing configuration and enrolled client keys. It composes `StoreValidator::canonical`,
`StoreKeyResolver`, `StorePeerAuthority`, the shared `CanonicalHandler` and its
per-physical-peer subscription factory. The optional HTTP listener uses that same
Service and Store. This is an explicit connect/2 runtime with no stock Core
fallthrough or protocol downgrade. The [default request and application commands](application-sdk.md) now use the
signed SDK and explicit HTTP APIs. The earlier high-level facade and other
workflows still await migration.

The protected server TOML names `bind`, `database`, `certificate`, `private_key`,
`signing_key_id`, `signing_algorithm`, `signing_private_key`,
`signing_key_not_before`, `signing_key_expires_at` and optional `http_bind` plus
`limits`. TLS and envelope signing use separate explicit key files. Signing keys
are PKCS#8 Ed25519 (`EdDSA`) or P-256 (`ES256`). Paths resolve against the config
file. Invalid/expired signing material is refused before installation recovery or
binding. The signing-key deadline closes the endpoint and owned peers and marks
the intake failed; changing keys requires an explicitly configured restart.

The trusted Rust process owner can now call
`RunningCanonicalServer::reload_tls_certificates().await` to load certificate
material from the original configured paths. TLS config clones used by canonical
QUIC/WT, WSS and record HTTPS share one certificate resolver. A successful
replacement preserves all served DNS/IP identities and advances a local
revision; an unchanged chain keeps the revision. Invalid, expired, mismatched,
oversized or narrower identity material leaves the last valid resolver active.
File validation runs off executor, with a short publication lock and no file IO
inside handshakes. Envelope signing and client trust retain their own configured
lifetimes and enrollment.

The [certificate rotation component](../evidence/gate-6/tls-certificate-rotation.json)
passes actual loopback TCP/TLS handshakes with normal CA/name verification:
new handshakes see the replacement, an existing session continues, and invalid
replacement retains the last valid identity. Four component/error functions pass.
Production QUIC reload also passes both envelope-signing algorithms, with six
full certificate-inspection handshakes and eight independent FFmpeg-decoded
before/after audio directions. Original runtime/generation/readiness bindings
stay fixed, fresh signed clients still authenticate, and both original media
offers remain the only rows at effect attempt one. Wrong names, unrelated roots
and missing intermediate chains still fail actual public native opening. All
owned shutdowns join. Actual WT/WSS/public HTTPS reload, operator CLI control,
public HTTPS routing and trusted proxy policy remain G6-053 work.

`session_reconnect_grace_seconds` sets the policy for future invitations (default
30, range 0–300 seconds). Each invitation freezes its policy. For a started
Session, confirmed loss of its last live Connection arms one durable deadline;
packet silence does not start the timer. Verified reconnection keeps the same SID
and clears the deadline. At expiry, a current permitted native offer/answer in
flight preserves the Session until it settles or loses authority. A previous
process, expired key, revoked credential, removed membership or narrower recipient
ceiling cannot extend it. Expiry freezes `session.ended` with reason code 408 and
`reconnect-grace-expired`; subsequent Connections require a new invitation.
Explicit `session.end` takes effect immediately. Started, participant and terminal
hints use one ordered durable source queue; each physical recipient is independently
authorized at delivery. Journal catch-up preserves the original fact and creates
fresh typed envelope IDs.

Pending TLS handshakes and established physical peers retain the same configured
connection slot. Incomplete handshakes expire after five seconds, incoming queues
are bounded and admission overload is refused before a new handshake. The
runtime owns its installation, intake, peer supervisor, idle-conversation worker
and HTTP facade. Shutdown fences installation authority, closes intake and peers,
and joins owned workers. Drop also retires authority and closes the endpoint.

Explicit client/server shutdown also awaits actual QUIC protocol drain. Quinn's
closing interval includes RTT variance and can outlast the worker-drain allowance.
The local profile uses a conservative allowance derived from the pinned estimator:
27 times observed smoothed RTT plus 250 milliseconds, bounded to 5–30 seconds.
The server uses the maximum observed peer RTT. This changes no worker deadline,
does not suppress drain failures and does not revoke an earlier operation receipt.

`connect_client::canonical::CanonicalClient` is the public signed Rust transport.
`CanonicalClientConfig::load` reads a protected client TOML with `endpoint`,
`server_name`, `trust_certificate`, `credential`, `identity_id`, `device_id`,
`device_kind`, `device_platform`, `signing_key_id`, `signing_algorithm`,
`signing_private_key`, `server_key_id`, `server_algorithm`, `server_public_key`,
`server_key_not_before`, `server_key_expires_at` and `timeout_ms`. The server pin is
a raw Ed25519 public key or uncompressed SEC1 P-256 public key, separately from
TLS certificate trust. Each client key and exact device descriptor must already
be enrolled through the provisioning API and bound to its current credential.
There is no implicit enrollment or trust-on-first-use.

`open` verifies signed challenge/session envelopes against that pin and checks
the selected profile, identified assurance, Identity and expiry. `send` accepts a
typed canonical `Payload` and `CommandScope`; it generates a fresh envelope ID
and timestamp and signs the complete validated envelope. The caller persists
logical operation IDs and intent before sending. A successful send proves local
submission only. `receive` verifies each original bounded envelope before exposing
it, including live facts and terminal assurance changes. Correlate `in_reply_to`,
resource scopes and the command family's response contract. A receipt, live fact
or partial response is not another request's ACK or a completed operation.

`keepalive` signs this peer's opaque session token and returns a fresh correlation
ID. Drive and correlate its ACK before the 30-second native idle boundary when
there is no other traffic. It cannot extend the current binding's credential or server-key expiry and does
not install refreshed authority.

`refresh_credential` and `refresh_credential_with_cancel` perform an explicit
signed `auth.refresh` on this same physical peer. The caller supplies the
successor bearer and separately provisions its current credential/device/signing
key binding; the SDK never issues, chooses or enrolls it. The server checks both
incarnations through Store authority, preserves owner and existing assurance provenance,
and rotates the opaque session token. The client accepts only the correlated,
verified `auth.session` for its original Identity and `connect/2`, checks the
unchanged assurance, a future expiry and a new bounded token, and bounds
usable expiry by the existing server-signing pin. Returned
`CanonicalAuthentication` contains Identity, assurance, expiry and profile only.
It never returns the session token or successor credential.

The exchange uses one absolute deadline bounded by the previous incarnation and
pin. Unrelated/live/streaming replies cannot extend it or install authority.
Malformed replies, cancellation during IO or an unknown outcome retire the peer;
reconnect explicitly using a known current credential instead of retrying an
uncertain refresh on a partial stream. A verified correlated refusal leaves the
previous local binding intact; actual Connect tests also verify its old keepalive
and ordinary commands remain usable after unbound/different-owner refusals.
Current scope after successful refresh is the successor's scope, even when it is
narrower; predecessor expiry does not retire the valid successor. Refresh is
separate from an assurance upgrade or correlated step-up, and ordinary command
methods continue to reject generic `auth.*` payloads.

The [refresh component evidence](../evidence/gate-6/auxiliary-auth-refresh.json)
records the complete current51-case client library and two actual listener/public
SDK cases (the positive case covers both EdDSA and ES256). This is one G6-046
component, not completion of vCon, auxiliary events or the whole task.

The [explicit SDK step-up exchange](step-up.md) returns a suspended command's
signed challenge without treating it as an ACK. The caller can supply a
successor bearer to `complete_step_up`; it collects the verified authentication
transition and the original command's replies without resending that command.
It retains the original deadline and operation/revision, keeps unrelated facts
available, and retires uncertain or contradictory transitions. A pending
challenge cannot be replaced by another batch or refresh. This currently uses
the caller-driven reader before owned native audio takes custody. Actual
Store-backed user consent and shared server policy are installed by the
[user-assurance component](user-assurance.md). Task-scoped delegation and
owned-audio transitions remain open G6-046 work.

The [durable record backend](connect-record-store.md) supplies Core with Connect
SQLite persistence and retained cross-process read custody. It is a storage
component. The [owned Session outcome worker](session-record-outcomes.md)
now finalizes original terminal metadata after media ends or the host restarts;
current source-authorized public record retrieval remains to be installed.

The [typed calls and bounded batch collector](canonical-calls.md) provide
Conversation, message, history, snapshot, subscription, read-report, Participant
and operation-status methods. The collector retains unrelated live facts in a
separate bounded inbox for `receive`.

Ordinary signed commands use a caller-driven reader and bounded inbox. Owned
native audio transfers that reader to its continuous signaling owner. Deadline expiry, cancellation or a partial-frame error
closes the physical peer, so a later read cannot resume halfway through an old
frame. Reconnect and recover/replay the same operation ID after uncertainty.
Client TLS and server signing trust are mandatory and independent.

The scoped runtime tests exercise the actual CLI/runtime composition and public
client, not a test-only profile driver. Their report records the exact coverage
that passed. Full high-level SDK migration, remaining assurance/step-up composition,
canonical media and remaining command/extension installation, underlying HTTP
write custody, Linux/browser/remote/provider profiles and Gate 6 acceptance remain
open. `serve` now selects this canonical runtime. The earlier `ConnectClient` and
other consumers of its config still await migration; their parity is not qualified.


Canonical consult, handoff and device replacement now enter the shared durable
transition admission path; see [canonical transitions](canonical-transitions.md).
The installed [general native media runtime](general-sessions.md) has production
signed-peer behavior evidence. The bounded audio-only [native consultation
coordinator](native-consultation.md) now applies actual private audiences; whole
consultation completion/provider/return work remains open. Admission-only dispatch
fixtures do not substitute for those physical media cases.

The [native network profile reference](native-network.md) describes explicit
Local/Remote address policy, configured TLS roots/chains, the complete opening
deadline and same-peer QUIC UDP rebinding. Qualification scope remains in its
source-bound evidence report.


Hosted HTTPS uses the optional `https` array. Omission preserves the simple
loopback profile. Each binding selects one surface; at most one of each surface
is allowed. HTTPS uses the same owned physical writer as protected local HTTP,
including issuance and record output custody. Live sockets and pending TLS
handshakes are bounded by configured connection capacity; incomplete handshakes
expire after five seconds.

| Surface | Routes | Admission |
|---|---|---|
| `application` | Canonical commands/discovery, named application APIs, records/files | Current bearer/source access; exact configured browser origins |
| `administration` | Provisioning, credential issuance, user consent and Task access | Existing per-operation issuer/permission rules; no browser Origin |
| `callbacks` | `/webhooks/telnyx` | Configured Telnyx verification public key and explicit route ceiling; current callback source authority |
| `health` | `/healthz` | Only bounded aggregate `{profile, ready}`; no private component/media details |

For an installed host, edit the protected server configuration created by init;
keep its signing and state settings. This fragment shows explicit exposure:

```toml
network_profile = "remote"
bind = "0.0.0.0:4433"

[[https]]
surface = "application"
bind = "0.0.0.0:8443"
public_origin = "https://connect.example.invalid:8443"
allowed_origins = ["https://app.example.invalid"]

[[https]]
surface = "administration"
bind = "127.0.0.1:8444"
public_origin = "https://admin.example.invalid:8444"

[[https]]
surface = "health"
bind = "127.0.0.1:8445"
public_origin = "https://status.example.invalid:8445"
```

Replace these example contacts with the deployment's exact contacts. Provision
its certificate chain/key at the protected configured paths before startup;
every HTTPS public name must be covered. A public origin alone opens no
additional interface. The callback surface additionally requires
`telnyx_callbacks.public_key` and a nonempty `telnyx_callbacks.routes` ceiling;
its verification key is provider public material. Account API credentials remain
server-side. Other installed provider adapters keep their explicit ingress.

An optional `trusted_proxies` list names exact immediate socket-peer IPs. A proxy
must overwrite the complete one-hop `X-Forwarded-Proto: https`,
`X-Forwarded-Host: <configured authority>` and numeric `X-Forwarded-For` tuple.
Keep Host equal to the configured public authority. The server strips this
metadata; current bearer, actor, assurance and resource authority still come
from the existing Store checks. Incomplete/ambiguous chains, RFC Forwarded,
port/prefix rewrites and asserted identity/provider-context headers are refused.
The default empty list refuses forwarding headers. HTTP3 WT remains on the
canonical QUIC UDP bind with its configured mount/Origins; WSS keeps its own
TLS listener/mount. The REST surfaces use HTTP/1.1 with guarded response custody.

After `serve` reports ready, SIGHUP explicitly reloads its original TLS paths.
Successful reload prints a local revision/change observation; failure logs a
bounded reason and retains last-good material. SIGTERM/SIGINT join shutdown.
Full handshakes use the current certificate resolver; established/resumed TLS
sessions retain their original negotiated identity and requests reauthorize.

The [HTTPS surface checkpoint](../evidence/gate-6/https-hosting-surfaces.json)
passes four new actual-listener/CLI functions plus ten current HTTP, issuance
and installation regressions. A fresh CLI also produces three schema-checked
HTTPS health/policy responses with ordinary CA/name validation. The initial
moved-config build and resumed-session certificate-inspection failure remain
failed evidence. Current WSS/HTTP3 reload/deployment composition and the G6-053
requirement audit remain open; no distinct-host/Linux/provider or full acceptance
qualification is inferred.


The current hosted composition additionally passes all seven hosting functions.
HTTPS, WSS and HTTP3 WT observe the shared replacement leaf in nine accepted
full handshakes across initial/rotated/last-good phases; foreign Origins refuse
both upgraded transports. Discovery keeps its configured contacts. A separate
Remote HTTPS policy fixture checks the exact published authority and wrong
TLS names on isolated loopback. Two raw sockets fill capacity, excess admission
refuses promptly, and five-second handshake expiry releases slots for reuse.
Every owned shutdown joins and no panic occurs. These WT/WSS probes check
routing and certificate lifetime, without authenticated control/media. Actual
Linux/distinct-host/browser-media/profile qualification remains separate.
