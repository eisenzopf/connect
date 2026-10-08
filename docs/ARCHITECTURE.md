# Connect architecture

Connect is a local communications server with a durable application model over
UCTP/QUIC. The same twelve operations are available through the Rust client,
Python/TypeScript native-helper SDKs, and an explicit loopback HTTP/JSON facade.
A loopback WebSocket parity listener exercises the same control profile; it is
not a native QUIC SDK or telephone media endpoint.

## Ownership

| Layer | Responsibility |
|---|---|
| connect-protocol | Profile, operations, outcomes, errors and limits; no transport/storage |
| connect-store | SQLite transactions, identity/credentials, immutable receipts, journal, provider jobs and recovery |
| connect-service | Authenticated operation dispatch and application authorization |
| connect-connector-api | Storage-independent message/voice requests and provider observations |
| connect-connectors | Telnyx/Vapi provider adapters and callback validation |
| connect-client | Rust native UCTP/QUIC authentication and request exchange |
| connect-server | Listeners, CLI lifecycle, explicit durable-job→adapter mapping and bounded workers |
| sdk/python, sdk/typescript | Public operations, validated types/results, helper/HTTP transports and stable errors |

The adapters do not import the store. They attempt provider actions and classify
rejection versus uncertainty; the server owns durable intent, retry decisions,
correlation and journal publication. Third-party adapters currently register in
server composition at build time; a dynamic plugin loader is not implemented.

## One operation

The transport authenticates a credential and hands a request to the service.
The store checks current authority and either commits effect+receipt+journal
atomically or rejects without acceptance. Replays use the actor's logical request
ID and return the original result only under current authority. Inbox notification
is a wake hint; SQLite is authoritative. Provider workers consume durable intents
and store observations. Unknown creation outcomes are not automatically repeated.

Worker shutdown cancels intake, drains every owned worker under a shared deadline,
reports returned errors/panics, and aborts stalled tasks. Cancelling the drain also
aborts its tasks. This is not a claim of automatic worker supervision/restart.

## Voice boundary

The verified route is handset ↔ Telnyx ↔ Vapi. Connect provisions, initiates,
observes, correlates and terminates provider calls; it does not carry their audio.
The assistant can answer while Connect is offline, and inbound discovery catches
up later. A SIP→Connect→Vapi WebSocket media bridge remains Gate 3 work. Local
QUIC audio probes demonstrate datagrams, not that complete telephone path.

Schema 6 is unchanged by the stabilization pass. Existing installations, backups,
provider credentials and remote resources are not modified by source checks.
Presence/contact preferences remain backlog decision B-001. Live SMS awaits
carrier approval. [Wire contract](../contracts/connect-control-v1.md) ·
[Contributor workflow](../CONTRIBUTING.md) · [Connector guide](CONNECTORS.md).
