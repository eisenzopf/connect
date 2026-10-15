# Gate 5 local package profile

Gate 5 candidates use the explicit `gate5-local` profile. They are unpublished
local macOS arm64 artifacts. Existing Gate 4 archives and the default
`gate4-local` builder/checker behavior remain historical evidence; changing a
profile label cannot qualify their additional Gate 5 behavior.

The [draft release notes](GATE5_RELEASE_NOTES.md) collect compatibility, unsupported
paths and source-scoped evidence. [Project-page copy](GATE5_PROJECT_COPY.md) is
prepared for review; neither document declares Gate 5 complete or published.

The runtime includes `connect`, `connect-example`, `connect-video`, Python and
TypeScript SDKs, the browser video facade and conversation reference client,
setup and connector guides, wire/contact/connector/VP8 media contracts, the independent
UCTP client and its pinned requirements/tests, and both connector examples. The native video guide and persisted public-SDK Session preparation helper accompany its executable.
The examples depend only on the included public connector API. Its package
manifest replaces workspace inheritance with the same dependency declarations;
Rust sources and tests remain byte-identical to the source archive. The
`connector-kit-source.json` file records these source bindings.

Native video source is included as a teaching asset; compile it from the complete source archive with its workspace manifest. The complete source archive supplies native video source and its codec/browser
fixtures, native secret fixtures, adapter/host integration tests, pinned upstream
patches and dependency locks, and the qualification scripts. The native video
example uses an explicitly installed FFmpeg for encoding. Browser qualification
requires an explicitly selected Chrome and Playwright installation. Neither
external tool nor their installation state is bundled. Camera acquisition is an
explicit example option. The server does not acquire the camera or spawn FFmpeg.

Prepare the two public recovery fixtures before building a candidate:

```sh
python3 scripts/build-recovery-baseline.py --schema 1 --public-source
python3 scripts/build-recovery-baseline.py --schema 16 --public-source
python3 scripts/package-local.py --profile gate5-local
```

The public baseline catalog fixes revisions and the schema1 initialization patch.
Schema1 is an identity-only recovery fixture, not an unmodified historical
messaging release. The separate schema16 fixture is an unmodified historical
Gate 4 source. Their executables and provenance manifests accompany the
qualification archive. Explicit `--previous-binary` and `--gate4-binary` paths
must match the same public catalog. Gate 4 retains its previous archived
milestone default. A real candidate requires a clean committed source tree and
checks it again after builds; packaging neither commits nor publishes source.

A source inventory rehearsal works offline against the current working tree:

```sh
python3 scripts/package-local.py --profile gate5-local \
  --rehearse-source .local/gate5/package-source-rehearsal.json
```

It checks required source assets and records their hashes, revision and dirty
state. It creates no release candidate and supplies no compile, installation,
media, setup or upgrade behavior evidence.

Verify an extracted candidate without invoking builds or Codex:

```sh
python3 scripts/check-package.py dist/CANDIDATE --profile gate5-local --inventory-only
```

This verifies all three archive digests, runtime file hashes, both historical
fixtures, the complete source inventory against the matching checkout, and
source-to-runtime contributor/reference/contract assets. Its result is
`inventory-verified`, with `whole_gate_qualified: false`.

For installed checks, select Codex explicitly and prepare the independent Python
environment from the pinned hashed requirements:

```sh
python3 scripts/prepare-independent-uctp.py
python3 scripts/check-package.py dist/CANDIDATE --profile gate5-local \
  --codex-binary /absolute/path/to/codex \
  --independent-python .local/independent-uctp/venv/bin/python \
  --node-binary /absolute/path/to/node \
  --ffmpeg-binary /absolute/path/to/ffmpeg \
  --chrome-binary /absolute/path/to/Chrome \
  --playwright-module /absolute/path/to/node_modules/playwright/index.mjs
```

The checker preserves retained installed baseline checks, runs both contributed
examples against the installed public API, and runs independent UCTP vectors and
control behavior against the installed Connect executable. Owned build phases
and first macOS loader startup have separate bounded budgets. Native video first
startup uses `--help`; it does not acquire or encode media.

The installed native lifecycle checks require seven SDK/CLI failure cases and an
eighth lane using the actual packaged `connect-video` executable. The native
example lane launches two distinct generated-source peers in one Conversation,
waits for complete incoming VP8 keyframes in both directions, and interrupts one
peer. A separate capture-child refusal run uses a private FFmpeg shim that
forwards generated video to the selected real codec and refuses the audio
`avfoundation` invocation before any hardware access. This controlled child
failure does not prove physical device permissions or camera/microphone capture.

Both peers must exit within five seconds with the expected interrupt or child
failure and counterpart terminal reasons. Forced cleanup, surviving owned
FFmpeg descendants, missing observations, changed input hashes or a lost
Conversation/Session fail qualification. After each fault, the same accepted
Session and exact message history must remain, ordinary communication must
continue, and two fresh peers must attach to that same Session and receive actual
keyframes before completing cleanup. These cases do not claim decoded frame
rate, bitrate or the 60-second video profile.

The installed driver runs this lane with the explicitly selected FFmpeg and
blocks completion when that tool is missing. Its `native_example_cleanup` result
is separate from the seven `installed_media_failures` cases and from full decoded
route qualification. To repeat the lane against explicit extracted inputs:

```sh
python3 /absolute/path/to/connect-source/scripts/check-native-video-cleanup.py \
  --runtime /absolute/path/to/extracted-runtime \
  --source-path /absolute/path/to/connect-source \
  --ffmpeg /absolute/path/to/ffmpeg \
  --source-revision CANDIDATE_REVISION \
  --work-root .local/native-example-cleanup \
  --output .local/native-example-cleanup.json
```

The selected runtime supplies `bin/connect` and `bin/connect-video`; the matching
source archive supplies the checker and public example sources. The report binds
both binaries, FFmpeg, Python, process-observation tools, source and helper hashes
before and after all two fault cases and both same-Session reattachments.

The new hooks execute fresh public CLI setup and failure recovery against the
installed executable, schema 1 and 16 migration plus selected current-schema
recovery and replay with the packaged Python SDK, and decoded video using the packaged browser assets/native executable.
Public setup also exercises twelve provider catalog, preview and readiness cases
through the same selected executable. An owned loopback Vapi fixture serves two
read-only discovery requests; Telnyx metadata is supplied locally. These cases
verify exact resource reuse, incomplete/duplicate catalog and purchase refusal,
capability and consent gaps, directional/stale readiness, explicit secret
reference selection and redaction. They perform no live provider requests or
resource changes, and the fixture ships in the source archive.
The connected-app journey selects the installed `connect` and `connect-example`
binaries and verifies both communication directions, downloaded artifact bytes,
unauthorized and ambiguous input refusal, original business and mutation IDs
after server/worker restart, receipt replay and one response per effect. Its
driver and helper hashes are checked again alongside both binaries. Normal
binary behavior remains separate from test-support lost-ack crash injection.
Installed Session controls run the selected server/helper with the packaged
Python and TypeScript SDKs over both native and HTTP transports. Nineteen CLI
receipts cover AV invite/accept/end/reject/cancel, wrong actor/Conversation and
changed-input conflicts; the SDKs originate sixteen more receipts. All thirty-five
receipts agree across the selected clients, and narrower replacement credentials
deny replay of all twenty-six previously successful operations. Six supervised
SDK phases require complete helper cleanup and scan retained diagnostics against
every issued credential. The source parity fixture exercises eight public entry points, including Rust, direct HTTP and MCP, and
remains separately scoped source evidence.
The installed reference check also requires its attributed AV decision and
artifact to survive restart, exact decision replay, one follow-up, explicit
context omissions and zero foreign-Conversation sources. No transcript or model
vision is inferred from AV transport.
Native Keychain library fixtures compile from the exact source archive and
separately exercise the selected installed CLI; the report distinguishes those
scopes. Tools and binaries are explicitly selected and hash-bound. A 60-second
decoded route pass cannot substitute for an unmeasured failure or operating case.

These hooks have source-level checks, but no Gate 5 candidate is yet qualified.
The checker returns `incomplete` (exit 2) for remaining acceptance observations
or missing independent/media tool inputs. Retained audio/task checks and a source
media pass cannot satisfy installed Gate 5 requirements. No ambient browser or
Python environment is silently substituted.
Private logs and fixture state remain under ignored `.local/package-checks`.
No profile claims whole Gate 5 closure or publication.


Installed reference qualification uses the selected runtime SDK/assets and
executable, then checks those input hashes again after cleanup. It exercises
keyboard/history/replay, verified file downloads and malformed/expired/foreign
files, narrow layout, and generated AV invite/reject/accept/mute/end/reinvite
through the actual reference UI. Its short AV journey is separate from full media
qualification. The installed media checks require all three clean 60-second
routes, all three impaired 60-second routes, and a separate 20-second
camera/microphone mute/recovery scenario on each route. Resource coverage and
invalid observer/derivation gaps remain blocking; the sole impaired A/V quality
availability exception is defined below. The exploratory two-call capacity rehearsal is
reported separately and does not expand the supported two-party profile.
The owner-approved October 15 impairment-only policy is
`connect-media-acceptance/2`; see [Decision 012](decisions/012-gate-5-impaired-media-proposal.md).
The media wire profile remains `connect-video-vp8/1`. Clean and mute scenarios
retain every strict quality check. In the unchanged impaired scenario only,
validly measured A/V skew and validly derived A/V measurement availability are
advisory. The raw `av_sync` result remains in each observation beside a separate
`av_acceptance` object (`policy`, `scenario`, `status`, `quality_status`, `reason`).
Malformed observer data, invalid derivation and source-identity failures remain
blocking, as do all existing non-A/V checks: identified duplex audio, the
existing impaired moving-video floor, continuous fresh motion, interruption
recovery, authorization, resource/network ceilings and paired cleanup.

Installed acceptance must require the matching new policy field and validate
its scenario; it must not require every impaired raw A/V result to say passed or
treat every raw failure as advisory. The two-second rule is fresh-motion recovery
after each interruption, not restoration of full clean quality. New-policy
qualification requires fresh checks and a matching identified candidate and
archive. Previous failures stay failed; neither this amendment nor selected
component results complete the installed lane, actual-model journey, human
walkthroughs or owner acceptance.

Each installed media report must contain exactly one completed observation for
each route with the expected scenario and full finite duration; an aggregate pass
cannot substitute for a missing, duplicated, failed or shorter route.
