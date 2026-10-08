# Connect

**Communication for people, apps, and agents.**

An open communications server and SDK from [Rudeless](https://rudeless.ai). One interface to reach out and be reached.

## Status

Connect is in development. Gate 1 is implemented and qualified locally on macOS arm64: native UCTP/QUIC, protected initialization, persistent identities, scoped credentials, transactional messaging, history/inbox recovery, and explicit backup/restore/upgrades. The Rust SDK and independent deterministic app/agent examples are tested. A local installable candidate and complete SDK source bundle are available in this workspace; no server or SDK release has been published. Gate 2 adds provider messaging, Python/TypeScript SDKs, HTTP/JSON access, and Vapi/Telnyx telephone control. Live Vapi chat, an outbound handset conversation, and inbound assistant answering are verified. Telephone audio runs directly between Telnyx and Vapi; a Connect-hosted SIP/WebSocket media bridge remains Gate 3 work. Live Telnyx SMS is deferred pending carrier approval. No full v0.1 release is claimed.

See [the documentation and delivery index](docs/index.html), [the Gate 2 product/delivery plan](docs/connect-gate-2-plan.html), [the alignment audit](docs/documentation-alignment.html), [the qualification result](docs/evidence/gate-1/result.json) and [local candidate instructions](docs/LOCAL_RELEASE.md). The capabilities below describe the complete product we intend to build.

## Local development

The project pins Rust 1.97.1 and audited upstream source revisions. Start with:

```sh
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-server -p connect-review-loop --bins
./target/debug/connect init --directory .local/demo --example review-loop
./target/debug/connect serve --config .local/demo/server.toml
# In another terminal:
./target/debug/connect health --config .local/demo/operator.toml
./target/debug/connect resolve --config .local/demo/reviews.toml connect:agent@local
```

See [the developer guide](docs/DEVELOPING.md) for the workspace layout and checks,
and [the implementation checklist](docs/connect-gate-1-implementation.html) for
progress and verification evidence. The source build and installed local candidate both pass the native messaging
journey. The sample agent is deterministic; no provider account is required.

## What we're building

Connect will let applications and AI agents exchange messages, share files, and communicate over voice and video through a shared API. The server will bring conversations, media, and connectors into one process. The SDK will make those capabilities straightforward to use.

An application will be able to use Connect to communicate outward and expose the same interface so other applications and agents can reach it.

The server and SDK come first. A reference client will let people try conversations, test capabilities, and see the SDK in use.

## A shared protocol

Connect will build on [rvoip](https://github.com/eisenzopf/rvoip) and [UCTP](https://rudeless.ai/uctp), with QUIC as the preferred native transport. Reliable streams will carry control traffic and datagrams will carry live media. UCTP will provide a common conversation model across transports.

Our ambition extends beyond this server: independent developers and service providers should be able to implement UCTP and communicate through the same open protocol.

## Connectors

Vapi, Telnyx, SIP, and WebRTC are the first integration priorities. We also want to connect Twilio, Vonage, WhatsApp, Telegram, RCS, and applications built by the community.

The storage-independent [connector API and example](docs/CONNECTORS.md) provide a starting point for contributing adapters. Each connector will expose the capabilities its service supports.

See [CONTRIBUTING.md](CONTRIBUTING.md) and the [architecture guide](docs/ARCHITECTURE.md) to work on the server or SDK.

## Help shape Connect

[Open an issue](https://github.com/eisenzopf/connect/issues) with a use case, connector idea, or protocol question. Tell us what you're building and who or what it needs to communicate with.

Connect grows from our work on [rvoip](https://github.com/eisenzopf/rvoip), [Thelve](https://rudeless.ai/thelve), and [Parley](https://github.com/eisenzopf/parley).
