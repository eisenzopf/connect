# Proposed Rudeless Connect project-page copy

Prepared locally for owner review. No site, release, download or remote content
has been changed. The following copy preserves the current unpublished status;
replace it only when the corresponding acceptance and publication are real.

## Page copy

**Connect**

**Communication for people, apps, and agents.**

Connect is an open communications server and SDK from Rudeless. Give an app or
agent a revocable identity, exchange messages and files, and keep the useful
history in one persistent conversation.

Start locally with two participants and no provider account. Applications use
public interfaces; agents can use MCP with an ordinary communication identity.
Task tracking and narrow grants are available when the workflow needs them.
The model host and its account remain explicit prerequisites for actual AI use.

The next local alpha adds a contributor connector kit, independently implemented
UCTP control, first-run setup and two-party video. Native participants use QUIC;
browsers use authenticated WebRTC. The video profile and matching installed
candidate are still being qualified for local macOS arm64.

Existing Vapi, Telnyx and SIP integrations have distinct capabilities and setup
requirements. Telephone is audio-only; an unsupported video route rejects
instead of silently becoming an audio call. Optional service accounts and
credentials stay separate from the first local exchange.

**Status: local alpha under qualification; unpublished.**

Explore the [source and documentation](https://github.com/eisenzopf/connect),
[read the local package profile](GATE5_PACKAGE.md), or
[share a use case](https://github.com/eisenzopf/connect/issues).

## Short conference description

Connect gives people, apps and agents one place to exchange messages, files and
conversation context. It combines a local server, public SDKs, revocable
identities and optional service connectors. The next macOS arm64 alpha is being
qualified for two-party VP8 video across native QUIC and browser WebRTC. The
project is open source and unpublished; the demonstration is not a claim that
all release acceptance is complete.

## Claims to retain when adapting the copy

| Wording | Why it matters |
|---|---|
| “Local macOS arm64” | Other platforms and public deployment have not been qualified by this gate. |
| “Under qualification; unpublished” | Component passes and a conference demonstration do not establish a matching installed release. |
| “Native QUIC; browser WebRTC” | The browser facade is not a browser-native QUIC implementation. |
| “Two-party VP8” | No multiparty video, screen sharing, SIP/PSTN video or Vapi vision is promised. |
| “Optional provider/model prerequisites” | Local deterministic messaging is account-free; actual models and external services have their own prerequisites. |
| “Explicit resume” when describing host continuity | A saved model session does not imply automatic wake, resume or an always-running host. |

Use the [draft release notes](GATE5_RELEASE_NOTES.md) for the compatibility matrix,
unsupported paths, component evidence and remaining acceptance. Link actual
immutable archives/checksums only after owner approval and publication; no
public download destination is invented here. When placing the page online,
point the relative documentation link at the approved repository documentation
or published documentation location.
