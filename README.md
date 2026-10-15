# Connect

**Communication for people, apps, and agents.**

An open communications server and SDK from [Rudeless](https://rudeless.ai). One interface to reach out and be reached.

## Status

[Gate 5 implementation](docs/connect-gate-5-implementation.html) is complete within
the approved local macOS arm64 scope:
three two-party VP8/PCMU video paths, a contributor connector kit, independently
implemented UCTP control, first-run setup and an expanded reference client.
Candidate `284cef1` has accepted installed qualification; 50 of 50 tasks and
16 of 16 criteria are complete within their recorded scopes. Usability and quality
review completion are owner-confirmed. Final owner acceptance is recorded;
see the [sealed candidate handoff](docs/GATE5_OWNER_HANDOFF.md). Review the [product plan](docs/connect-gate-5-plan.html)
and [local package profile](docs/GATE5_PACKAGE.md) for the qualification boundary.
[Draft release notes](docs/GATE5_RELEASE_NOTES.md) and
[proposed project-page copy](docs/GATE5_PROJECT_COPY.md) are prepared locally for review.

The retained Gate 4 baseline is ready for local macOS arm64 owner review: [36 implementation
items](docs/connect-gate-4-implementation.html#tracking) and [14 acceptance
criteria](docs/evidence/gate-4/acceptance.html) have recorded evidence. Agents
get [one revocable communication identity](docs/AGENT_ACCESS.md) across their
authorized conversations. Tasks track work when needed; narrow grants remain
optional. The [Codex host](docs/CODEX_TASK_HOST.md) preserves its session for
explicit resume.

The frozen `709cf82` candidate passes [42 offline qualification phases](docs/evidence/gate-4/offline-709cf82.json),
[17 installed-package checks](docs/evidence/gate-4/package-709cf82.json) and
[32 actual-model MCP calls](docs/evidence/gate-4/real-model-agent-709cf82.json).
The [public journey runner](docs/AGENT_JOURNEY_QUALIFICATION.md) makes the
separate model check reproducible. [Hosted CI](https://github.com/eisenzopf/connect/actions/runs/38334742398)
uses `d64f1b9`; its workflow and three qualification-fixture files are the only
changes from the immutable runtime candidate. The [qualification record](docs/evidence/gate-4/qualification-controls-d64f1b9.json)
keeps that boundary and the earlier failed attempts explicit.

Historical [Vapi](docs/evidence/gate-4/actual-vapi-chat-dc73996.json),
[voice/chat](docs/evidence/gate-4/live-voice-continuity.json) and
[SMS](docs/evidence/gate-4/sms-0737-rehearsal.json) rehearsals retain their exact
source, account, handset and topology limits. The [source/provider review](docs/evidence/gate-4/review-remediation-709cf82.json)
explains how those proofs relate to the current code.

The local alpha is owner-approved and release-ready, and remains unpublished.
Gate 5 qualifies local macOS arm64. Additional platforms need separate qualification;
community publication follows a separate owner instruction.

See [the documentation and delivery index](docs/index.html) and
[local candidate instructions](docs/LOCAL_RELEASE.md). The capabilities below
describe the complete product we intend to build.

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
progress and verification evidence. The retained Gate 4 source build and installed local candidate both pass the native messaging
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
