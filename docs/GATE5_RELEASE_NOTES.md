# Gate 5 local alpha — release notes prepared for review

This is a draft for the local macOS arm64 candidate `0d285d168509`. Gate 5 is
still under qualification. No complete installed qualification, final acceptance
or publication is asserted here. The retained Gate 4 candidate and historical
provider rehearsals keep their original source and topology limits.

Follow the [implementation tracker](connect-gate-5-implementation.html),
[acceptance map](gate-5-acceptance.json), [unsealed acceptance progress](evidence/gate-5/acceptance.html)
and [package guide](GATE5_PACKAGE.md) for
current evidence. A conference demonstration, component test or short media
rehearsal does not establish the full release profile.
The [product roadmap](connect-roadmap.html) and [feature coverage map](feature-coverage.json)
retain all F-01–F-21 commitments, including the paths still unsupported.

The [retained baseline](evidence/gate-5/baseline.json) now reproduces all 42
original source phases through matching-source individual runs and all 17
original installed checks. Original failures remain recorded; this is composed
coverage, not a single uninterrupted source run or a changed Gate 5 candidate pass.

## Current candidate observations

Candidate `0d285d168509` passed archive inventory and independent archive review.
Its selected archived reference check passed all 11 general checks and all 13
AV/continuity checks. A separate private real-browser augmentation passed five
contact-selection checks against archived assets and the public local API.
The [contact repair record](evidence/gate-5/contact-selection-repair.json) retains
all three original regression failures, the 216-test source result and the
selected browser followup. These checks do not constitute a complete installed
pass; see the exact [package evidence](evidence/gate-5/local-package-qualification.json).

A separate 60-second [two-native-pair observation](evidence/gate-5/native-concurrency-observation.json)
recorded roughly 5.1–5.3 independently decoded changing frames per second across
four participants, below the 24 fps clean-profile comparison. This exploratory
result does not extend the supported **one simultaneous two-party call** profile
or establish a capacity claim. Approved browser impairment remains unresolved.
Actual-model execution, three independent human walkthroughs and owner acceptance
remain open. All 16 acceptance criteria remain not-qualified. Earlier failed
runs retain their original outcomes. These progress annotations postdate the
0d archive and are not claimed to be its baked documentation.

## Changes prepared for this candidate

- Two-party VP8 video with duplex PCMU audio through native QUIC/native QUIC,
  native QUIC/browser and browser/browser routes. Browsers use authenticated
  WebRTC. The reference client adds invitation, separate camera/microphone mute,
  teardown and new invitation flows; the native example uses a public SDK and
  explicitly selected external FFmpeg.
- A public connector API, manifests, normalized lifecycle/receipt/provenance
  contracts, two contributed examples and an offline conformance harness.
  Independent incoming contact is an actual new turn with attributable reply
  context. Uncertain provider acceptance retains its original logical ID and is
  reconciled without another create/send.
- A separate Python UCTP reliable-control implementation, including its own
  framing, authentication, request correlation and QUIC transport. This is
  independent control evidence; it does not implement video or audio.
- Provider-free setup, native contact pairing, protected server secret
  references, explicit provider resource selection and recoverable setup intents.
  Contact ownership, Conversation authority and SMS enrollment remain separate.
- Source/schema-aware recovery and a package driver that requires the installed
  runtime, matching public source, external tools and retained failure cases.
  Current source uses additive schema 18. Schema 16 and known schema 17 source
  state have separate migration fixtures; no historical schema-17 binary is
  implied by the synthetic fixture.

## Compatibility and current qualification boundary

“Component verified” below refers to the linked source-scoped report. It does
not qualify a later binary, full operating profile or installed package.

| Surface | Behavior and compatibility | Evidence / remaining boundary |
|---|---|---|
| Messages, files and tasks | One persistent Conversation; stable mutation IDs, durable receipts and current authority on replay. Ordinary revocable identity remains the default; narrow task grants are optional. | Retained [Gate 4 acceptance](evidence/gate-4/acceptance.html); matching Gate 5 release rerun required. |
| Rust, Python, TypeScript, HTTP, CLI and MCP | Shared control model. Python/TypeScript native clients use the Rust helper; HTTP is separate. MCP is a control surface, not a media-byte API. | [SDK guide](SDK.md), [source contact parity](evidence/gate-5/contact-interface-parity.json); broader installed/model parity remains open. |
| Three two-party AV routes | VP8 video plus PCMU audio; a transient Connection ending preserves the accepted Session and Conversation for fresh authorized attachment. Explicit Session end is terminal. | [Media contract](../contracts/connect-media-vp8-v1.md), [component transport checks](evidence/gate-5/video-transport.json); full decoded route/impairment/mute qualification remains separate. |
| Native video example | Generated sources use real external codecs. SIGINT and controlled capture-child refusal require paired cleanup, preserved history and fresh same-Session attachment. | [Native guide](../examples/native-video/README.md), [installed cleanup integration guards](evidence/gate-5/package-native-cleanup-integration.json). Guards are not actual installed process proof. |
| SIP / telephone | Audio-only Sessions remain supported. An AV `sip_dial` rejects with `unsupported-media-profile`; it cannot silently downgrade. An explicit audio-only Session can share the same Conversation. Already owned legs retain cleanup authority. | [Admission/race tests](evidence/gate-5/video-admission-cleanup.json); no new carrier-live proof. |
| Existing Vapi / Telnyx routes | Default carrier transport paths retain their documented behavior. New normalized message extensions and `with_observed_driver` voice composition are explicit opt-ins. A manifest alone does not activate those hooks. | [Connector guide](CONNECTORS.md), [source audit](evidence/gate-5/provider-source-audit.json); historical live results are not fresh Gate 5 provider qualification. |
| Late replies and SMS tracking | A cancelled/expired task's late answer stays ordinary unassigned communication. Verified delivery-only observations can be recorded after STOP/lease expiry; replies and new dispatch still require current policy. | [Connector host proof](evidence/gate-5/connector-runtime-integration.json); no task continuation, new outbound effect or policy resurrection is authorized. |
| Connector contributors | Storage-independent public kit and deterministic fixture harness. Provider credentials, verified callback origin and current route authority remain required where applicable. | [Independent contribution](evidence/gate-5/contributor-exercise.json), [API/host proof](evidence/gate-5/connector-runtime-integration.json); fixture results do not establish carrier/account interoperability. |
| Independent wire implementation | Seven reliable control operations over actual QUIC; no Connect SDK/helper/parser imported. | [Conformance guide](PROTOCOL_CONFORMANCE.md), [control-only proof](evidence/gate-5/independent-conformance.json); matching installed candidate rerun required. |
| Setup and secrets | Local demo needs no provider/model account. Keychain references or an explicitly selected environment reference; missing/locked storage fails without a plaintext fallback. | [Setup guide](SETUP.md), [source setup/recovery](evidence/gate-5/setup-upgrade.json), [vault migration](evidence/gate-5/secret-vault-migration.json); final installed candidate checks remain separate. |
| Upgrade / rollback | Stop the server and retain a matching verified backup; `serve` does not migrate implicitly. Restore into a new directory with a compatible binary. Database schema is authoritative during setup metadata recovery. | [Public historical recovery](evidence/gate-5/migration-recovery.json), [package prerequisites](GATE5_PACKAGE.md); current installed migration/restore qualification remains required. |

The connector report records 287 API/Store/connector tests, 24 host tests and both
40-case harnesses at its recorded source boundary. The admission report records
14 Store tests. The installed-cleanup integration records 19 pure Python package
guard tests. These totals overlap other suites and must not be summed into a
release-wide count. Consult each report's source hashes and retained failed
attempts before reusing it for a later source graph.

## Profile targets and unsupported paths

The [frozen local profile](decisions/011-gate-5-profile.md) targets one two-party
640×360 VP8 call, at most 30 fps, with at least 24 independently decoded moving
frames per second for each clean 60-second route. It separately requires duplex
identifiable audio, A/V marker alignment, impaired recovery, bounded resources,
concurrent verified file transfer and paired teardown within five seconds.
These are qualification requirements, not measurements claimed by this draft.
Queue occupancy is sampled; Quinn's internal evictions/queued age and physical
speaker/display latency are not observed by those counters.

| Outside this candidate's advertised scope | Boundary |
|---|---|
| Windows, Linux, Intel macOS or hosted deployment | Additional platforms/topologies need separate qualification. No hosted relay or public ingress deployment is included. |
| Multiparty video, screen sharing, H264/AV1 or transcoding | The selected profile is two-party VP8/PCMU only. |
| SIP/PSTN video, Vapi vision or SMS media | Unsupported routes reject before media admission. Telephone remains audio-only; SMS remains policy-controlled messaging. |
| Browser native QUIC | Browser media uses WebRTC; native QUIC is the Rust/native-helper path. |
| Independent client media or MCP media bytes | Independent client is reliable control only; use the documented native/browser media SDKs for media. |
| Physical camera/microphone or arbitrary NAT/TURN reachability inferred from generated tests | Generated codec/capture-refusal fixtures do not prove physical device permission, public network reachability or production capacity. |
| Automatic model resume or autonomous host startup | Exact saved host sessions use explicit resume. Model/host prerequisites and actual inference are qualified separately. |
| Universal connector/provider support or telephone ownership implying SMS consent | Each adapter declares its supported operations and configuration. Native pairing does not enroll a number or enlarge authority. |
| A ten-minute onboarding guarantee | The target requires the agreed independent fresh-user walkthroughs; an author's script is not that evidence. |

## Source, licenses and distribution

Connect ships the repository [MIT license](../LICENSE). The current
[dependency source lock](../dependencies/source-lock.json) records the exact
rvoip and Parley Git revisions and MIT licenses, plus the checksummed
`rtc-dtls`, `rtc-interceptor` and `quinn-proto` crate archives with their upstream
revision provenance and MIT OR Apache-2.0 licenses. It also records every applied
patch and SHA256 digest. Modified upstream behavior is attributed to those
patches rather than described as unmodified upstream code. The package notice
builder includes both license texts for each prepared crate override as well as
the Git and registry notices; [third-party notices](../THIRD_PARTY_NOTICES.md)
explain these inventories. The unpublished 0d285d168509 candidate passed archive
inventory and independent archive review; see the exact
[package evidence](evidence/gate-5/local-package-qualification.json). Its runtime,
source and qualification archives retain that source boundary. Thelve and Bridgefu
are design provenance, not copied source in this candidate.

The independent Python client has [pinned dependency notices](../examples/independent-uctp-client/DEPENDENCIES.md)
and hashed requirements. FFmpeg, Chrome, Playwright, Node and the independent
Python environment are selected external prerequisites, not bundled tools.
Their installed versions/builds and licenses remain separate from Connect's
repository license. Later source repairs require a new matching archive and
affected checks; the existing inventory result does not close release acceptance.

Before release, seal a clean source revision and matching runtime/source/
qualification archives, verify their manifests/checksums and included notices,
complete the remaining accepted checks and independent review, and record owner
review. Publication, public download verification and project-page changes
remain a separate owner-authorized action. The [proposed project copy](GATE5_PROJECT_COPY.md)
is local preparation only.
