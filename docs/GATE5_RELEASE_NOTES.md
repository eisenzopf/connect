# Gate 5 local alpha — release notes prepared for review

These locally prepared release notes describe the unpublished macOS arm64
candidate `ab843b843fa7`, source
`ab843b843fa7445d4d6c663939c7226c5c432c7d`. Its complete installed qualification,
actual-model journey and six bounded provider compatibility claims are
independently accepted. Three independent fresh-user walkthroughs, final
documentation and quality review, release sealing and owner acceptance remain
open. No final Gate 5 acceptance or publication is asserted. Historical results
keep their original source and topology limits.

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

The [designated candidate](evidence/gate-5/release-candidate.json) passed exact
archive inventory and independent archive review. The
[complete installed result](evidence/gate-5/complete-installed-ab843b8.json)
passed 18 baseline checks and all 26 Gate 5 phases, including all nine native/
native, native/browser and browser/browser clean, impaired and mute conditions.
Both strict runtime inventories passed without additions, missing files,
changed bytes or symlinks; source/tool inputs remained stable and owned process
groups were empty after cleanup.

All 18 observer acceptance decisions passed `connect-media-acceptance/2`.
Raw A/V quality remains separate: 15 passed and three impaired observations
retain failed quality measurements, accepted only as advisory under Decision
012. Clean and mute A/V checks remain strict, and no malformed observer or
non-A/V hard failure was waived. The observed maximum native codec emission
hold was 198.788917 ms against 200 ms; maximum impaired fresh-motion recovery
was 1.950659 seconds against two seconds. These are observations of this run,
not guarantees for arbitrary machines or networks.

The separately accepted [actual-model journey](evidence/gate-5/actual-model-ab843b8.json)
used `codex-cli 0.162.0-alpha.17.2`: 28 successful MCP calls across the initial
and explicitly resumed turns, zero tool/turn errors, two verified artifact byte
payloads, and the same saved host session after server restart. One regular
credential served two authorized conversations; source-backed followup and
independent incoming contact were verified without duplicate effects. This
proves neither automatic wake nor Vapi inference or model video understanding.

The historical `0d285d168509` reference/contact proof and separate 60-second
[two-native-pair observation](evidence/gate-5/native-concurrency-observation.json)
remain source-specific. The latter measured about 5.1–5.3 changing fps across
four participants and does not extend the supported one simultaneous two-party
call profile. Earlier failed packages and media runs retain their original
outcomes in [package evidence](evidence/gate-5/local-package-qualification.json).

These release materials and acceptance annotations postdate the immutable
ab843 archives. They are a separate review supplement, not a claim that the
archived documentation contains these later results. G5-047 remains open for
independent human feedback and final documentation review.

## October 15 impairment-only policy amendment

The owner approved “Relax quality expectations under poor network conditions”.
[Decision 012](decisions/012-gate-5-impaired-media-proposal.md) records the exact
qualification policy `connect-media-acceptance/2`, separate from the unchanged
`connect-video-vp8/1` wire/media profile. Only impaired A/V skew and validly
derived A/V measurement availability become advisory degraded-quality results.
Raw measurements and failure reasons remain visible. Clean and mute checks stay
strict; malformed observers, invalid derivation, wrong source identity and all
existing non-A/V hard checks still fail. The 3% loss, 0–100 ms delay/reordering,
two 250 ms interruptions and existing 1 fps impaired moving-video floor remain
unchanged, alongside identified duplex audio, continuous fresh motion, bounded
resources and five-second paired teardown.

Recovery within two seconds still means fresh decoded motion after each
interruption; it does not promise restoration of full clean quality. Candidate
`0d285d168509` predates this policy implementation. It and earlier failed reports
are not retrospectively qualified. The matching ab843 installed run now supplies new-policy qualification;
it does not change any historical result. Policy approval alone was not a pass.
Actual-model evidence is separately accepted; human and owner acceptance remain
separate requirements.

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
| Messages, files and tasks | One persistent Conversation; stable mutation IDs, durable receipts and current authority on replay. Ordinary revocable identity remains the default; narrow task grants are optional. | Retained [Gate 4 acceptance](evidence/gate-4/acceptance.html); [ab843 installed proof](evidence/gate-5/complete-installed-ab843b8.json). |
| Rust, Python, TypeScript, HTTP, CLI and MCP | Shared control model. Python/TypeScript native clients use the Rust helper; HTTP is separate. MCP is a control surface, not a media-byte API. | [SDK guide](SDK.md), [source contact parity](evidence/gate-5/contact-interface-parity.json); [installed](evidence/gate-5/complete-installed-ab843b8.json) and [actual-model](evidence/gate-5/actual-model-ab843b8.json) proof retain their distinct scopes. |
| Three two-party AV routes | VP8 video plus PCMU audio; a transient Connection ending preserves the accepted Session and Conversation for fresh authorized attachment. Explicit Session end is terminal. | [Media contract](../contracts/connect-media-vp8-v1.md), [component transport checks](evidence/gate-5/video-transport.json); all nine decoded route/condition checks pass in the [ab843 installed result](evidence/gate-5/complete-installed-ab843b8.json), with raw impaired quality retained separately. |
| Native video example | Generated sources use real external codecs. SIGINT and controlled capture-child refusal require paired cleanup, preserved history and fresh same-Session attachment. | [Native guide](../examples/native-video/README.md), [installed cleanup integration guards](evidence/gate-5/package-native-cleanup-integration.json). The [ab843 installed result](evidence/gate-5/complete-installed-ab843b8.json) separately passes actual installed cleanup and deadline-boundary behavior. |
| SIP / telephone | Audio-only Sessions remain supported. An AV `sip_dial` rejects with `unsupported-media-profile`; it cannot silently downgrade. An explicit audio-only Session can share the same Conversation. Already owned legs retain cleanup authority. | [Admission/race tests](evidence/gate-5/video-admission-cleanup.json); no new carrier-live proof. |
| Existing Vapi / Telnyx routes | Default carrier transport paths retain their documented behavior. New normalized message extensions and `with_observed_driver` voice composition are explicit opt-ins. A manifest alone does not activate those hooks. | [Connector guide](CONNECTORS.md), [source audit](evidence/gate-5/provider-source-audit.json); [six accepted bounded claims](evidence/gate-5/provider-claims-ab843b8.json) do not assert fresh provider execution. |
| Late replies and SMS tracking | A cancelled/expired task's late answer stays ordinary unassigned communication. Verified delivery-only observations can be recorded after STOP/lease expiry; replies and new dispatch still require current policy. | [Connector host proof](evidence/gate-5/connector-runtime-integration.json); no task continuation, new outbound effect or policy resurrection is authorized. |
| Connector contributors | Storage-independent public kit and deterministic fixture harness. Provider credentials, verified callback origin and current route authority remain required where applicable. | [Independent contribution](evidence/gate-5/contributor-exercise.json), [API/host proof](evidence/gate-5/connector-runtime-integration.json); fixture results do not establish carrier/account interoperability. |
| Independent wire implementation | Seven reliable control operations over actual QUIC; no Connect SDK/helper/parser imported. | [Conformance guide](PROTOCOL_CONFORMANCE.md), [control-only proof](evidence/gate-5/independent-conformance.json); [matching installed control proof](evidence/gate-5/complete-installed-ab843b8.json) accepted; media is outside this independent implementation. |
| Setup and secrets | Local demo needs no provider/model account. Keychain references or an explicitly selected environment reference; missing/locked storage fails without a plaintext fallback. | [Setup guide](SETUP.md), [source setup/recovery](evidence/gate-5/setup-upgrade.json), [vault migration](evidence/gate-5/secret-vault-migration.json); [matching installed checks](evidence/gate-5/complete-installed-ab843b8.json) passed; actual provider readiness is excluded. |
| Upgrade / rollback | Stop the server and retain a matching verified backup; `serve` does not migrate implicitly. Restore into a new directory with a compatible binary. Database schema is authoritative during setup metadata recovery. | [Public historical recovery](evidence/gate-5/migration-recovery.json), [package prerequisites](GATE5_PACKAGE.md); [ab843 installed historical recovery](evidence/gate-5/complete-installed-ab843b8.json) passed; rollback still requires compatible verified backup/binary inputs. |

The connector report records 287 API/Store/connector tests, 24 host tests and both
40-case harnesses at its recorded source boundary. The admission report records
14 Store tests. The installed-cleanup integration records 19 pure Python package
guard tests. These totals overlap other suites and must not be summed into a
release-wide count. Consult each report's source hashes and retained failed
attempts before reusing it for a later source graph.

## Six bounded provider compatibility claims

The [independent claim review](evidence/gate-5/provider-claims-ab843b8.json)
accepts only the following wording. Historical live receipts keep their own
sources; current local evidence does not establish new live provider execution.

| Surface | Accepted scope | Excluded without separate authorized evidence |
|---|---|---|
| Telnyx SMS and signed callbacks | Historical delivery/HELP/STOP/START remains source-specific. Current request/status mapping, single-attempt dispatch, replay, eligibility and stricter malformed-input admission are locally verified compatibility composition. | Current-candidate handset delivery, carrier callback execution or actual account/enrollment readiness. |
| Vapi text and MCP tools | Historical actual model/tool proof plus current SDK/Store/Worker loopback tests supports default request/context/tool-binding compatibility. Builtin messages retain the legacy receipt path. | Current Vapi model response, actual tool discovery/execution and source-backed response on this candidate. No normalized used-source provenance is inferred for builtin Vapi. |
| Vapi telephone control | Historical live proof plus current legacy start/status/end/recovery fixtures supports default control composition; Worker::start selects normalized=false. | Newly advertised normalized Vapi telephone execution. with_observed_driver is explicit and its carrier execution is not proved by fixtures. |
| Native/browser Vapi audio and SIP/PSTN | Historical audio remains source-specific. Current audio bridge, codec, authorization, teardown, pressure and upstream tests can establish local compatibility of the changed graph. Shared runtime is not unchanged. | Exact-candidate carrier answer/audio/remote-end behavior, arbitrary network reachability or production interoperability. |
| Provider setup, catalogs and secrets | Local fixtures establish selection, refusal, protected-reference resolution, uncertainty recovery and metadata-only readiness reasoning. | Actual account readiness, ownership, SMS consent, provisioning or purchases; no such operation follows from local proof. |
| Normalized connector extension and contribution | Fixture connector and trusted-host lifecycle, receipt binding, reconciliation, independent ingress and attribution semantics. | An additional real carrier/model capability or normalized builtin-provider execution; no automatic activation is inferred. |

## Profile targets and unsupported paths

The [approved local profile](decisions/011-gate-5-profile.md) targets one two-party
640×360 VP8 call, at most 30 fps, with at least 24 independently decoded moving
frames per second for each clean 60-second route. It separately requires duplex
identifiable audio, strict clean/mute A/V marker alignment, impaired fresh-motion
recovery, bounded resources,
concurrent verified file transfer and paired teardown within five seconds.
Impaired A/V quality is measured and advisory only under the versioned policy
above. The accepted installed run satisfies these requirements under policy 2; its raw measurements and limits remain in the linked report.
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
explain these inventories. The unpublished ab843b843fa7 candidate passed archive
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

## Immutable candidate identity and remaining release work

The archive base name is
`connect-0.1.0-alpha.1-macos-arm64-ab843b843fa7-gate5-local`.
Verify `SHA256SUMS` before extracting into a new directory; the
[candidate designation](evidence/gate-5/release-candidate.json) binds these exact
archives and their matching source revision:

| Archive suffix | SHA-256 |
|---|---|
| `.tar.gz` (runtime) | `151a7c7eac8a426ad0741dbe4e5fd1a522681daad90f30c9df3f204466088b36` |
| `-source.tar.gz` | `6c2438afd000dfb0ebc29523023ef01df796477d1b1abec0de9628989d233b4a` |
| `-qualification.tar.gz` | `6303271b566d8b06aa801a84b01546d3b98e1e9f5df87bac083c57a66b70b1a0` |

The [installed release record](evidence/gate-5/installed-release.json) binds the
accepted package behavior. Human walkthroughs, their documentation corrections,
final independent quality/seal and owner acceptance remain required. Publication,
public download verification and project-page changes are separate actions.
The [proposed project copy](GATE5_PROJECT_COPY.md) remains local preparation only.
