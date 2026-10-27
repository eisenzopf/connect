# Connect

**Communication for people, apps, and agents.**

An open communications server and SDK from [Rudeless](https://rudeless.ai). One interface to reach out and be reached.

## Build with Connect

Start with a local conversation, then add addressed messages, retained history
and live media. Your app owns its goals and approval policy; Connect provides
the authorized communications boundary.

<!-- CONNECT:NAV:START -->
[Install](docs/start/index.html#install) · [First message](docs/start/index.html#first-message) · [Voice and video](docs/reference/index.html#voice) · [Handoff](docs/reference/index.html#handoff) · [Parley tutorial](docs/tutorials/parley.html) · [Concepts](docs/concepts/index.html) · [Hosting](docs/hosting/index.html) · [SDK and HTTP reference](docs/reference/index.html) · [Contribute](CONTRIBUTING.md) · [Engineering](docs/engineering/index.html)
<!-- CONNECT:NAV:END -->

The [developer documentation](docs/index.html) separates the published local
release from current development source. The [Parley progression](docs/tutorials/parley.html)
shows how chat, calls, consultation, human takeover and follow-up fit together.
Its shared scenario and deterministic fixtures are available; the installed app
and runnable tutorial checkpoints remain under implementation.

## Choose the matching version

**[Connect v0.1 is published](https://github.com/eisenzopf/connect/releases/tag/v0.1)**
with version `0.1.0` binaries for local macOS arm64, SDKs, examples, complete
source, recovery fixtures, checksums and a qualification supplement. All six
release assets were independently downloaded over public HTTPS and rehashed.

Read its [release notes and disclosed exceptions](docs/releases/v0.1.md) and
[matching installation guide](docs/LOCAL_RELEASE.md). No remote-server deployment
or live provider operation is part of this release.

Current Gate 6 source has a breaking pre-alpha API. The typed Rust SDK has local
source qualification; unified browser/Node/Python packages, remote hosting and
the full application journey remain in progress. Browser and native private-audio
findings remain open. Use [current source references](docs/reference/index.html)
and their named scopes; published v0.1 instructions do not qualify this source.
Plans, historical releases and proof are in [Engineering](docs/engineering/index.html).

## Local development

Use the repository's pinned Rust toolchain and the
[development prerequisites](docs/DEVELOPING.md). Local source setup currently
targets macOS arm64. Prepare the audited dependencies before Cargo:

```sh
brew install opus pkgconf
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-server --bin connect
./target/debug/connect init --directory .local/demo --example review-loop \
  --bind 127.0.0.1:4433 --http-bind 127.0.0.1:4434
./target/debug/connect serve --config .local/demo/server.toml
# In another terminal:
./target/debug/connect health --config .local/demo/operator.application.toml
./target/debug/connect resolve --config .local/demo/reviews.application.toml connect:agent@local
```

This enrolls communication identities with separate native and HTTP configuration.
It does not run the older review-loop app, whose signed SDK migration remains
open. See [current bootstrap](docs/developer/canonical-bootstrap.md) and
[the workspace guide](docs/DEVELOPING.md). For the published provider-free
message/reply/history demo, use [its matching first run](docs/start/index.html#first-message).

## A shared protocol

Connect builds on [rvoip](https://github.com/eisenzopf/rvoip) and
[UCTP](https://rudeless.ai/uctp). Reliable control, negotiated media and durable
application state have separate responsibilities. See the
[shared vocabulary](docs/concepts/index.html) and [protocol reference map](docs/reference/index.html).

Our ambition extends beyond this server: independent developers and service providers should be able to implement UCTP and communicate through the same open protocol.

## Connectors

Vapi, Telnyx, SIP, and WebRTC are the first integration priorities. We also want to connect Twilio, Vonage, WhatsApp, Telegram, RCS, and applications built by the community.

The storage-independent [connector API and example](docs/CONNECTORS.md) provide a starting point for contributing adapters. Each connector will expose the capabilities its service supports.

See [CONTRIBUTING.md](CONTRIBUTING.md) and the [architecture guide](docs/ARCHITECTURE.md) to work on the server or SDK.

## Help shape Connect

[Open an issue](https://github.com/eisenzopf/connect/issues) with a use case, connector idea, or protocol question. Tell us what you're building and who or what it needs to communicate with.

Connect grows from our work on [rvoip](https://github.com/eisenzopf/rvoip), [Thelve](https://rudeless.ai/thelve), and [Parley](https://github.com/eisenzopf/parley).
