# Connect

**Communication for people, apps, and agents.**

An open communications server and SDK from [Rudeless](https://rudeless.ai). One interface to reach out and be reached.

## Status

Connect is in development. Gates 1–3 have recorded local macOS arm64 qualification for durable messaging, scoped credentials, recovery, provider control, native QUIC audio, SIP/PSTN and browser WebRTC media, bounded files, availability and cross-channel context. Rust, Python and TypeScript SDKs and HTTP/JSON share the application contract. The [pre-Gate 4 entry evidence](docs/connect-pre-gate-4-entry.html) records the follow-up repairs, reviewed SMS conversation, native SDK context recall and HELP/STOP/START suppression/recovery on the named test lane. That evidence retains its original account, handset and topology limits.

Gate 4 adds MCP communication tools and a supplied agent skill, with [one
revocable communication identity](docs/AGENT_ACCESS.md) as the default local
installation. The agent uses its key across authorized conversations; tasks can
track work, replies, waits and effects. Narrow task grants remain an optional
mode. The [Codex host](docs/CODEX_TASK_HOST.md) preserves its exact session with
explicit resume. Vapi retains verified call/chat bindings.

Gate 4 is complete for local macOS arm64 owner review: all 36 implementation
items and 14 required criteria have source-linked evidence. The [frozen local
`0.1.0-alpha.1` package](docs/LOCAL_RELEASE.md) passes 17 installed checks outside
the checkout. A fresh clean checkout of the same source passes all 41 required
offline phases, including strict Clippy, Rustdoc, SDK, upstream, operating and
recovery checks.

The [actual normal agent](docs/evidence/gate-4/real-model-agent-dc73996.json)
uses one key across two conversations, preserves draft bytes, resumes its exact
session after restart and answers independent incoming contact. Its 30 MCP
calls succeed without task-grant renewal. [Fresh Vapi text](docs/evidence/gate-4/actual-vapi-chat-dc73996.json)
invokes three successful source-backed tools. Earlier [Connect-carried
voice/chat](docs/evidence/gate-4/live-voice-continuity.json) and [reviewed SMS](docs/evidence/gate-4/sms-0737-rehearsal.json)
proofs retain their exact source versions and limits; the [source reconciliation](docs/evidence/gate-4/final-source-composition.json)
links unchanged provider paths and current shared-MCP checks. Failed attempts
remain recorded.

Review the [completed checklist](docs/connect-gate-4-implementation.html#tracking)
and [qualification report](docs/evidence/gate-4/acceptance.html). The development
schema is 16; upgrades preserve existing state and require a matching verified
backup. This alpha is unpublished. Full v0.1 video, complete connector kit,
independent protocol conformance, first-run/reference experience, broader
platforms and community publication remain Gate 5 work.

See [the documentation and delivery index](docs/index.html), [the Gate 2 product/delivery plan](docs/connect-gate-2-plan.html), [the alignment audit](docs/documentation-alignment.html), [the qualification result](docs/evidence/gate-1/result.json) and [local candidate instructions](docs/LOCAL_RELEASE.md). The capabilities below describe the complete product we intend to build.

## Local development

The qualified development host is macOS arm64 with Rust 1.97.1, Node 22+,
Python 3.11+, Apple command-line developer tools and Homebrew. Install the native
Opus dependency before Cargo; the audited upstream sources are prepared next:

```sh
brew install opus pkgconf
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-server -p connect-review-loop --bins
./target/debug/connect init --directory .local/demo --example review-loop
./target/debug/connect serve --config .local/demo/server.toml
# In another terminal:
./target/debug/connect health --config .local/demo/operator.toml
./target/debug/connect resolve --config .local/demo/reviews.toml connect:agent@local
```

See [the developer guide](docs/DEVELOPING.md) for the workspace layout and checks,
and [the gate documentation index](docs/index.html#gates) for
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
