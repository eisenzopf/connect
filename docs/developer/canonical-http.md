# Current canonical HTTP control

`POST /v2/commands` accepts the normative UCTP v2 JSON envelope. It uses the
same service command parsers, durable operation engine, reply builders and
bounded error mapping as the signed native listener. See the generated
[OpenAPI reference](../../contracts/openapi-canonical-http.json).

HTTP uses an explicit current bearer and `connect/2` response profile. It does
not invent native signature, Device or physical-peer evidence. The native path
still requires its enrolled Device/key, signatures and retained peer authority.
Both paths check the current Identity, Conversation Participant, original source
audience, granular permission and protocol floor. Actor fields are assertions.
No stock Core handler can create another authoritative Connect Conversation.

```rust,no_run
# async fn example() -> connect_client::Result<()> {
use connect_client::{canonical::CanonicalHttpClient,
    application::ApplicationClientConfig, conversations::CreateRequest, MessageDraft};
let config = ApplicationClientConfig::load(
    std::path::Path::new(".local/signed/reviews.application.toml"))?;
let mut client = CanonicalHttpClient::open(config).await?;
let mut input = CreateRequest::persistent(vec![]);
input.tenant_id = Some("tnt_default".into());
let room = client.create_conversation("saved-create", input).await?;
let receipt = client.send_message("saved-send", &room.id,
    "saved_message", MessageDraft::text_all("Review this draft")).await?;
assert_eq!(receipt.message_id, "saved_message");
# Ok(()) }
```

The HTTP and native clients share their typed control implementations. Neither
client selects another transport after failure. HTTP has a 64 KiB encoded request
budget, 2 MiB response budget, at most 128 reply envelopes and one absolute
transaction deadline including body decoding. Cancellation or an uncertain write
does not replace the caller's saved logical IDs. Reconnect or choose the other
explicit client and replay those IDs under current authority.

| Command family | Current behavior |
|---|---|
| Conversation create/list/get/close | One durable model, current views and original mutation receipts. |
| Participant add/update/remove | Current authority and expected revision; roles remain labels. |
| Message send/history/read | Original message IDs/audiences, current visibility, checked actor claims and original recipient receipts. |
| Conversation subscribe | Native live events; HTTP requires explicit `live:false` for a finite snapshot/cursor catch-up. No automatic live downgrade. |
| Operation get/cancel/reconcile | Exact asserted resources, current owned sources and original protected evidence. |
| Consult/handoff/device replacement | Durable admission and installed native coordinators; configured telephone replacement also has actual private confirmation and movement. Status distinguishes preparation, known completion and uncertainty; see [telephone replacement](telephone-replacement.md). |
| General Session control | Invite/accept/reject/cancel/update/end share durable receipts and current appearances. Present HTTP actors also project an empty checked Core Session; see [general Sessions](general-sessions.md). Connections and media require separate admission. |

For a parsed request, HTTP returns canonical reply envelopes with the original
`in_reply_to`, a matching batch `request_id`, resource scopes and an explicit ACK
or error terminator. HTTP status matches the terminal error code. Authentication
or parse failure before an envelope is available returns the same bounded public
problem vocabulary without fabricating correlation. Replies are `no-store`;
private input, credentials and raw storage/provider errors are not echoed.

Cursor facts carry their original Session/Connection/operation source scopes,
which the SDK verifies against the inner immutable journal event. Those source
IDs are not a new operation requested by the subscription. Notifications and
receipt hints remain hints; the journal is authoritative.

The SDK application adapter and server application entry points use one closed
classification for genuine application-specific calls. They refuse canonical
communication, including old provider call/SIP mutations, over the older
generic Operation/`connect.request` dispatch.
Named application, task and trusted administration APIs remain subject to their
own current Store authorization; they cannot become a fallback canonical path.

General Session control now includes creation and explicit invitation/acceptance
for joins, leave/role/end and unconnected capability decisions. A completed HTTP
decision with a current present actor projects the same disposable checked Core
Session as signed control, while retaining the original command fence. It creates
no physical peer, admitted media Participant, Connection or media reader. A later
signed peer can attach to that Session under its own current authority. Terminal
decisions retire the container; replay of an older invitation cannot recreate it.
Physical connection and stream orchestration still require their installed
coordinators and separately checked media negotiation. Uninstalled requests fail
explicitly.
G6-037's normative coverage assignment owns shared routing and error/ACK
correlation, not those later orchestration outcomes. Linux, remote/native/browser
operating profiles, independent conformance, release/performance and publication
remain separate requirements.


`session.consult.return` uses its own current named permission and a fresh logical
operation ID. Scope it to the original Conversation, Session and primary Connection,
with the current Session topology revision. Its payload names the original
`consultation_record_id` and `retained_connid`. Status distinguishes admission,
actual confirmed restoration and observed private Connection retirement; replay does
not repeat an attempted effect. The final result includes `remote_continuing`, so
known remote loss cannot be mistaken for a continuing call. See the
[transition contract](../../contracts/connect-transitions.md#native-authorized-return-g6-039-implementation-in-progress).

The separately scoped named application reads `context_view` and
`context_deliveries` use `/v1/operations` through `ApplicationClient`. Their
[context and receipt reference](../../contracts/openapi-context.json) includes
captured native destinations and control problems/lifecycles. They do not create
a canonical command fallback or grant physical media/provider authority.

Protected [public assistant hosting](public-assistant-host.md) can react to an
AI's own committed `session.accept` using the same HTTP/signed Session parser.
The reply still confirms Session admission; separately observed provider context
and actual native activation determine readiness. No HTTP command installs a
provider profile or grants its model execution authority.

Canonical startup can now load explicit public native Vapi hosts from server TOML.
The named `participant_route` and capability reads use the same constructed
Endpoint/credential/key/secret bindings as hosting admission. Unhosted Endpoints
sharing a provider reference remain unconfigured, and current key revocation is
visible before provider I/O. See [startup configuration](public-assistant-host.md#canonical-startup-configuration).

Protected [canonical provider messaging](canonical-provider-messages.md) composes
Vapi chat and Telnyx SMS on the same Store and signed/HTTP message commands.
Account and verified Endpoint selection belong to server configuration. Telnyx
callbacks use a separate narrow listener and the [callback OpenAPI](../../contracts/openapi-provider-observations.json).

Private incoming-contact administration is the named
`POST /v2/incoming-contacts` API on the main administrative listener. It uses a
closed list/get/resolve/reject command set and current administrator/original
account-owner authorization before JSON decoding or decision replay. Browser
Origin and provider call/chat context headers are refused; callback-only ports
do not expose it. Metadata is not a Conversation history grant. See the
[incoming-contact HTTP reference](../../contracts/openapi-incoming-contacts.json)
and [routing/SDK contract](incoming-contact-routing.md). These administrative
commands do not enter the canonical communication envelope dispatcher.

## Record discovery

`conversation.get` includes authorized `record` snapshot items with `pending`,
`ready` or `unavailable` state. The SDK `conversation_records` method returns a
bounded page; drain its next-page token. Discovery requires `conversation.get`
and `record_get`, current Session source access and the frozen membership epoch
or explicit Session history share. `session_get` is independent. Only ready
items include UUID/revision; no canonical digest, storage URL, record body,
private source/count or captured-audio assertion is exposed.

Finite `conversation.subscribe` catch-up and native live subscriptions emit `recording.vcon-ready` before
its authorized `record_ready` cursor event. The hint names UUID/revision and is
not future authority. New snapshots recover state after restart, expiry or a
changed record. HTTP `recording.vcon-fetch` uses a closing HTTP/1.1 response that
retains current source authority and exact record custody through actual socket
IO. See [http-record-retrieval.md](http-record-retrieval.md) for bounds, expiry,
cancellation and current reference-delivery limits. Native retrieval is documented
in [native-record-retrieval.md](native-record-retrieval.md).

When an original physical effect remains uncertain, operator reconciliation returns
HTTP 409 with category `state` and reason `reconciliation_required`. Preserve the
original target and decision IDs, inspect the retained evidence and obtain
attributed settlement. This is distinct from an invalid command or permission
denial; declared completion cannot create physical evidence.

Native QUIC listeners and clients use the explicit
[local/remote network profile](native-network.md), configured TLS trust and
verified server identity. The plaintext application HTTP listener remains
loopback; native Remote configuration does not expose it.

## Canonical transport discovery

`CanonicalHttpClient::transport_discovery` reads authenticated
`GET /v2/discovery` on the canonical application listener. It returns the
`connect/2` profile, control-envelope and physical-peer limits, and the configured
WebTransport HTTPS and TLS WebSocket WSS contacts (each `null` when disabled).
Each contact requires an exact allowed Origin; WebTransport datagram availability
is negotiated per session. WSS discovery describes control hosting; it does not
assert WebRTC media readiness. Install
TLS trust and the enrolled server signing pin separately before opening it.

The GET body must be empty. Current bearer authentication and the shared HTTP
concurrency budget apply before body collection. Responses use `no-store`.
The SDK bounds response collection to 8192 bytes, retains its existing absolute
deadline and cancellation behavior, rejects unknown/incompatible descriptors,
and performs no automatic retry. Discovery creates no Session, Endpoint
verification, trust enrollment or media grant. An unrelated application listener
returns unavailable; callback-only listeners expose no discovery route.
See the [HTTP/OpenAPI reference](../../contracts/openapi-canonical-http.json).

In Local mode, an omitted advertised WebTransport URL preserves the actual bound
IPv4 or bracketed IPv6 address. Fresh Local certificates cover localhost and both
loopback IP identities. Remote mode still requires an explicit HTTPS contact.
See [WebTransport composition and rehearsal](canonical-webtransport.md) and
[WSS control hosting](canonical-websocket.md).
