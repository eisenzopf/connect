# Run a local Connect candidate

## Gate 5 candidate entry point

The designated `gate5-local` candidate is `284cef16c64b`, source
`284cef16c64b324ce2add037b013249692a60087`, for local macOS arm64.
It is unpublished. [Exact archive hashes and designation](evidence/gate-5/release-candidate.json)
and the [notice-package composition](evidence/gate-5/package-notice-composition-284cef1.json)
bind this immutable candidate. New archive, installed-version and provider-free
setup checks passed, followed by [complete installed qualification](evidence/gate-5/complete-installed-284cef1.json):
18 baseline checks, all 26 Gate 5 phases and all nine media conditions, with
both strict inventories passing. All three binaries and 240 protected runtime
assets match ab843 exactly. The separate accepted 28-call actual-model journey
is [composed evidence from ab843](evidence/gate-5/actual-model-composition-284cef1.json),
not a new inference run on 284cef. Independent
human walkthroughs, final documentation/quality/seal and owner acceptance
remain open; this is not final Gate 5 acceptance. Its three executable artifacts are `bin/connect`,
`bin/connect-example` and `bin/connect-video`; the matching source archive,
SDKs, reference client, contracts and qualification inputs have distinct
verification obligations. Use the [Gate 5 package guide](GATE5_PACKAGE.md) and
[draft release notes](GATE5_RELEASE_NOTES.md) for the current compatibility matrix
and exact explicit tool inputs. [Project-page copy](GATE5_PROJECT_COPY.md) is
prepared locally for owner review, not deployed.

Verify the candidate archives/checksums and inspect its manifest before use.
Current Gate 5 source uses schema 18; select the actual candidate schema and
source revision from its manifest. The retained schema-16 Gate 4 binary and
known schema-17 source fixtures are separate recovery inputs. The schema-17
fixture is not a claim that a historical schema-17 executable was released.
`serve` does not migrate implicitly: stop the affected server, retain its
matching verified backup and use explicit migration. Restore/rollback uses a
new directory and a compatible binary; see [setup recovery](SETUP.md) and
[historical-input qualification](GATE5_PACKAGE.md).

For a first run, replace `CONNECT` with the selected candidate's absolute
`bin/connect` path and follow [the setup guide](SETUP.md). The provider-free
`setup demo` exchanges local messages/files and verifies restart/history with
deterministic participants. It is separate from actual-model, reference-video
and installed-release qualification.

Optional server provider secrets use protected references selected through
`CONNECT_SECRET_REFERENCES`; macOS Keychain and an explicitly selected
environment reference are supported. Existing installations without that
reference file retain their documented environment configuration. Missing or
locked storage never authorizes a plaintext fallback. Do not move provider keys
into client/host configurations. [Setup](SETUP.md) describes vault migration and
recovery without repeating an uncertain provider creation.

FFmpeg, Chrome, Playwright and Node are explicitly selected external tools;
the independent Python environment is prepared separately. They are not bundled
with Connect. Generated codecs and controlled capture-child refusal do not
prove physical camera/microphone permissions. The [accepted installed media report](evidence/gate-5/complete-installed-284cef1.json)
records 18 passing policy decisions: 16 raw A/V quality passes and two valid
impaired advisory quality failures. Clean/mute and all non-A/V hard checks remain
strict. [Provider compatibility](evidence/gate-5/provider-claims-ab843b8.json)
is limited to six reviewed statements; no new carrier or Vapi execution is
inferred. Independent fresh-user walkthroughs and owner acceptance remain open.
These updated instructions postdate the candidate archives; no archive was
modified to include this later review supplement.

The [retained baseline](evidence/gate-5/baseline.json) now reproduces all 42
original source phases through matching-source individual runs and all 17
original installed checks. Original failures remain recorded; this is composed
coverage, not a single uninterrupted source run or a changed Gate 5 candidate pass.

## Retained Gate 4 local candidate runbook

The instructions and evidence below describe the historical Gate 4 candidate.
Its schema-16 and installed/model/provider claims remain tied to their named
sources. They do not qualify the current Gate 5 candidate.

This candidate packages the Connect server, native Rust helper, Python and
TypeScript SDKs, task/MCP tools, workflow skill, agent-presence example, browser
facade, and the human conversation test client for
macOS arm64. It is unpublished and performs no remote deployment. Provider-live
evidence is recorded separately from the package qualification.

The default local agent installation uses [one regular revocable identity
credential](AGENT_ACCESS.md) across authorized conversations, with optional task
tracking. [Scoped task installation](TASK_ACCESS.md) remains available for a
narrow delegation. The [709cf82 candidate](evidence/gate-4/package-709cf82.json)
passes all 17 required installed checks, with no failures or omissions. Earlier
package reports retain their recorded sources and modes. The
[Gate 4 review](evidence/gate-4/acceptance.html) records completed local
qualification and the evidence limits for owner review. This remains an
unpublished review candidate.

## Verify and install

Verify all three archives in the candidate directory:

```sh
shasum -a 256 -c SHA256SUMS
```

Extract the runtime archive into a new `releases/` directory. Keep state outside
the extracted release. Use the absolute path of `bin/connect` below; no PATH,
daemon, DNS, tunnel, or existing installation change is required.

Earlier local checks observed a pause before a new executable started; its
cause was not established. Package qualification records the first `--version` launch
separately with a 180-second installation budget and owned-process cleanup.
It then repeats the version command under the ordinary 15-second CLI budget;
service, MCP and media deadlines are unchanged. A first-launch timeout fails
qualification and remains recorded. Do not disable system security or retry a
communication with a new logical ID to work around installation startup.

## Start a private local installation

Replace `CONNECT` and `EXAMPLE` below with the installed absolute paths of
`bin/connect` and `bin/connect-example` respectively.

In terminal 1:

```sh
CONNECT init --directory state --example review-loop --bind 127.0.0.1:4433
CONNECT serve --config state/server.toml --http-bind 127.0.0.1:8080
```

Choose free loopback ports before initialization when those ports are occupied.
Initialization preserves a completed installation and refuses an incomplete
destination. Configurations, credentials, the certificate/private key, SQLite
database, and artifact blobs are private files. Never paste credentials into
commands, logs, or issue reports.

In terminal 2:

```sh
CONNECT health --config state/operator.toml
CONNECT diagnostics --config state/operator.toml
EXAMPLE reviews --config state/reviews.toml
```

In terminal 3:

```sh
EXAMPLE agent --config state/agent.toml
```

Both example processes use only the native SDK. Each has its own work database
for handled events, logical request IDs, replies, and inbox cursor. Stop and
restart either worker with the same files to exercise durable continuation.

## Gate 4 task and agent workflows

The [709cf82 local candidate](evidence/gate-4/package-709cf82.json) packages
source revision `709cf824295c12963b5d62214877b91fcd750e8d` and passes all 17
installed checks. The [fresh normal-agent actual-model proof](evidence/gate-4/real-model-agent-709cf82.json)
uses one regular key across two conversations, verifies draft bytes and source
facts, resumes the exact session after server restart, handles a tracked reply
and answers independent contact. Its two fresh model turns record 32 successful
MCP calls and zero failures. Two local preparation commands read the installed
skill and calculate literal draft bytes/digests; they are retained in the private
records. The independent review passes all 49 checks, including actual MCP
receipts, downloaded artifact bytes, full histories and the exact saved session.
Model communication uses the installed MCP tools; independent observations
use the installed public SDK. The public [model-journey runbook](AGENT_JOURNEY_QUALIFICATION.md)
provides the separately invoked check. Run `scripts/live/agent-journey.py` from
the matching source archive, using the installed runtime as its explicit input.

A tracked follow-up uses the received reply journal event to authorize and
correlate its effect. That reference does not consume the reply implicitly.
The current normal MCP tools expose explicit handled-reply consumption; the
model uses the current revision after the effect is confirmed and before
completion. An independent public-SDK observer verifies the retained attribution
and consumed marker. The unanswered sibling task remains waiting.

The [implementation plan](connect-gate-4-implementation.html#G4-033) and
[Gate 4 review](evidence/gate-4/acceptance.html) link the completed current
qualification: [42 required offline phases](evidence/gate-4/offline-709cf82.json),
17 installed checks, and separately recorded actual model behavior. The
workspace suite passes 591 tests with no failures; its nine ignored cases are
explicitly executed in named complete-run phases. The run begins from an
independent fresh clean `709cf82` Git clone with verified pinned dependency
sources. Only generated Cargo cache and verified historical baseline binaries
are reused; no application state or credentials are copied. All three
[hosted jobs](https://github.com/eisenzopf/connect/actions/runs/38334742398)
also pass under CI configuration
`d64f1b944368f5c763d7872272e983dd7c10fede`: default-feature source, Gate 3/schema-6
recovery and all-feature Gate 4 source. The [qualification controls](evidence/gate-4/qualification-controls-d64f1b9.json)
record the exact PR merge checkout and its tree equality to that branch,
with fixture revision `7bdd5341db558f062102df54487b8c2ad77573ff` recorded separately.
Only the workflow and three qualification-fixture files change; all 856 other
recorded inputs match `709cf82`, including the runtime, SDKs, dependencies and
Rust tests. The load fixture observes HTTP 100 Continue for each admitted
incomplete body before probing saturation. It retains 32 slots, 512 KiB
bodies, the shared two-second setup bound, body timeout and recovery assertions,
and the original profile budgets.

The original combined `709cf82` job retains its 90-minute cancellation after
its source checks passed. Its separate Gate 4 job retains the failed admission-
saturation assertion. Controlled loopback scheduling reproduces that fixture
race on the unchanged `709cf82` executable; the independent review distinguishes
reproduced cause from unavailable per-request observations on the deleted CI
runner. The independent recovery job at `7bdd534` separately timed out during
cold compilation, before any fixture ran. The final workflow adds an explicit
30-minute `--no-run` preparation step for that same locked/default-feature
executable; it claims no test execution. The original 600-second fixture phase,
all later checks and the 90/90/360-minute job budgets remain unchanged. Local
command verification used a warm cache; the final hosted receipts separately
verify cold preparation and actual subsequent behavior. Later green CI changes
none of the prior failure or cancellation results. CI remains account-free and does not qualify an installed model host
or live communication provider.

Historical [normal-agent dc73996](evidence/gate-4/real-model-agent-dc73996.json)
retains 30 successful calls and its corrected-observer boundary. Historical
[scoped Codex draft/reply](evidence/gate-4/real-model-draft-7edb1b1.json) and
[incoming contact](evidence/gate-4/real-model-incoming-7edb1b1.json) retain their
`7edb1b1` sources. None substitutes for the new regular-key/explicit-consume
proof. The prior [clean dc73996 offline run](evidence/gate-4/offline-dc73996.json)
remains a separate 41-phase result; interrupted or failed attempts keep their
original outcomes.

The historical [Vapi text proof](evidence/gate-4/actual-vapi-chat-dc73996.json)
on `dc73996` discovers seven optional provider-bound scoped tools and completes
source recall and one useful cited send with three successful model calls and
zero failures. [Connect-carried voice/chat](evidence/gate-4/live-voice-continuity.json)
on `596e9b7` and [reviewed SMS](evidence/gate-4/sms-0737-rehearsal.json) on
`9abf38f` retain their account, handset, source and topology limits. The
[current source/provider review](evidence/gate-4/review-remediation-709cf82.json)
compares the retained provider components and records the changed SIP terminal
classification. Its 26 focused tests cover both callback/cleanup orders,
active hangup, failed cleanup and real local SIP cancellation in default and
all-feature profiles. This is local SIP evidence; it does not claim a new
carrier call, SMS rehearsal or Vapi cloud turn on `709cf82`. The earlier
[source composition](evidence/gate-4/final-source-composition.json) remains
archived unchanged.

Archive documents retain their packaging checkpoint, including earlier
`dc73996` status links carried by the immutable `709cf82` archives. The current
source-specific reports and these updated instructions live in the repository;
they do not change archive bytes, manifests or recorded hashes. Use the matching
archived checker for package reproduction. This candidate remains unpublished.
The [full product roadmap](connect-roadmap.html) keeps video, the complete
connector kit, independent protocol conformance, first-run/resource setup,
the complete human reference client, broader platform qualification and
owner-reviewed v0.1 publication in Gate 5.

The runtime carries [task contracts](../contracts/connect-task-v1.md),
[protected grant setup](TASK_ACCESS.md), the [workflow skill](../skills/connect/SKILL.md)
and [Codex installation/resume instructions](CODEX_TASK_HOST.md). The server
exports the matching embedded skill and private host configuration. The
[default agent setup](AGENT_ACCESS.md) issues one regular communication credential;
tasks optionally track work without per-task issuance or five-minute renewal.
Individual configured communication actions need no approval prompts. A narrow
task export remains a separately selected installation.

The [incoming-contact example](../examples/agent-presence/README.md) runs from
`examples/agent-presence/codex-inbox.py` with `PYTHONPATH` set to the installed
`sdk/python` directory and `--executable` set to installed `bin/connect`.
This example driver uses the optional narrow-task host. Use a private application
state directory and the documented explicit-contact policy. One invocation
processes one contact; later continuation uses the saved host session. The
regular-key host uses the separate [agent-access setup](AGENT_ACCESS.md) and can
inspect its authorized inbox through the same connection. Neither host adapter
is an automatic model scheduler. Task/conversation history survives restart;
every new operation still checks current authority.

Package qualification runs its test harness from the extracted source archive,
while the application, Python SDK and native executable must load from the
extracted runtime. Source-path fallback fails the checks. An explicitly selected
Codex executable uses scripted loopback Responses and empty account directories;
these checks make no inference or live-provider claim. Contributor tests and
provider simulators are in the source archive. The runtime includes the usable
application and skill assets.

## Gate 3 interfaces

The installed native SDK exposes persistent conversations, bounded Sessions,
availability, context, artifacts, native PCMU audio, and SIP leg control. Python
and TypeScript use the installed `connect` helper for UCTP/QUIC control and PCMU
audio. HTTP provides authenticated control and raw artifact bytes; it is not an
audio transport. The browser facade uses WebRTC and a configured loopback
signaling listener; it does not claim browser-native QUIC.

The runtime includes `examples/conversation-client`. Serve that directory from
a loopback origin and point it at the configured HTTP/WebRTC listeners. It uses
the public browser SDK for history, messages, artifacts, availability, Session
control, microphone input, and remote audio. Browser playback and microphone
failures are surfaced to the user.

SIP and WebRTC listeners are disabled unless configured. Copy the commented
sections in `config/local.example.toml` into the private server config and use
credentials and durable IDs created for that installation. An inbound route can
create a bounded Session, or its optional `session_id` can join one already
accepted audio Session. Caller ID and SIP headers never select authorization or
merge conversation history.

For an outbound carrier trunk, configure `[sip.outbound]` with its loose-route
proxy, asserted SIP identity, authentication username, and the *name* of the
environment variable holding its Digest password. The server reads that value
only at startup and fails closed when it is missing or empty. For the current
Telnyx credential-connection profile, the starting values are
`sip:sip.telnyx.com;lr` and an identity URI built from the assigned E.164 number.
Keep this private operator configuration separate from durable conversation
authority. A wildcard SIP listener also requires `media_bind_ip` naming a
concrete local interface for RTP allocation; the public `advertised` and
`media_public` addresses are SDP/signaling mappings and cannot be used as local
bind addresses unless they are assigned to the host. Public inbound SIP/RTP
still requires a reachable advertised address and UDP media ports; an HTTPS
callback tunnel does not provide either.

Live Vapi and Telnyx credentials stay in environment variables and are never
part of this archive. The account-free qualification uses loopback provider,
SIP, WebRTC, and native peers. A local loopback pass does not prove public PSTN,
NAT, TURN, or provider reachability.

## Backup, restore, and upgrade

Create a verified backup before migration:

```sh
CONNECT backup --config state/operator.toml --server-config state/server.toml --output backups/pre-upgrade
CONNECT restore --backup backups/pre-upgrade --directory restored --bind 127.0.0.1:4434
CONNECT serve --config restored/server.toml
```

Backup manifest version 2 covers the SQLite snapshot and referenced artifact
blobs. Restore requires a new directory and verifies both before publication.
App-owned work databases and separately issued credentials remain their owners'
recovery material.

`serve` never migrates implicitly. Stop the affected server, retain its matching
verified backup, then run:

```sh
CONNECT migrate --config state/operator.toml --server-config state/server.toml --backup backups/pre-upgrade
```

The current Gate 4 source migrates supported schemas 1–15 to schema 16, adding
durable Vapi task-session setup bindings. Older task-linked legs receive an
unknown-intent marker and must continue with a new voice leg on the same task
and conversation. This preserves their history without guessing lost setup
credentials. The [schema-16 709cf82 candidate](evidence/gate-4/package-709cf82.json)
passes installed walkthrough, Vapi host, SDK and recovery checks. The
[Gate 4 review](evidence/gate-4/acceptance.html) links the completed historical
executable recovery and current backup/restore qualification. Check the installed
manifest for its exact source and
schema; older Gate 3 archives use schema 10. A live Session becomes
interrupted across restart; Connect preserves durable intent and never claims a
socket or media pump survived. Roll back by restoring the matching pre-upgrade
backup into a new directory and using its compatible binary.

## Reproduce qualification

The source archive contains the full workspace, lockfile, patches, contracts,
tests, and dependency source lock. The separate qualification archive contains
the historical schema-1 milestone binary and provenance manifest, without a
database or credentials.

Complete source qualification requires a Git checkout with full history. To
reproduce the frozen `709cf82` candidate, check out its exact revision rather
than substituting newer main:

```sh
git clone https://github.com/eisenzopf/connect.git connect-source
cd connect-source
git checkout --detach 709cf824295c12963b5d62214877b91fcd750e8d
brew install opus pkgconf
bash scripts/prepare-dependencies.sh
```

The source host requires Rust 1.97.1 through rustup, Node 22+, Python 3.11+ and
Apple command-line developer tools. Selected-host qualification additionally
requires an explicit installed Codex executable. The recorded host is
`codex-cli 0.162.0-alpha.17.2`; use the
[official installation guide](https://learn.chatgpt.com/docs/codex/cli), inspect
`--version` and `exec --help`, and select the actual absolute path. Generic
installation is not proof that another version has the qualified MCP/JSONL and
exact-resume behavior. The account-free checks exercise those interfaces without
model authentication; actual inference is a separate owner-authorized rehearsal.

From that matching full-history checkout, with the extracted historical
qualification archive outside the source directory:

```sh
python3 scripts/check-gate-4.py --bounded-workspace --codex-binary /absolute/path/to/codex
bash scripts/check-local.sh --previous-binary /absolute/path/qualification/connect-schema1
```

For archive and installed qualification, extract the source archive and invoke
its own matching checker. It does not require the extracted source to be a Git
repository; it prepares the pinned dependency sources and verifies the selected
installed runtime, SDKs and archived schema-1 binary:

```sh
python3 /absolute/path/to/extracted/connect-source/scripts/check-package.py \
  /absolute/path/to/candidate-directory \
  --codex-binary /absolute/path/to/codex
```

The checker compares source bytes, including the workflow, to the candidate's
source archive. Running a frozen `709cf82` archive check through a newer main
checkout can correctly fail that comparison. For a new candidate, use its own
recorded revision and matching source. Archive documents retain their packaging
checkpoint; the separate owner-review supplement does not replace runtime or
source bytes.

The separate strengthened hosted qualification uses checkout
`d64f1b944368f5c763d7872272e983dd7c10fede` and
`python3 scripts/check-gate-4.py --source-only`. That account-free source lane
includes the admission fixture from `7bdd534`; its recovery workflow also
prepares the first linked fixture before running the behavior checks. This is a
distinct qualification source, so its checker must not replace the frozen
`709cf82` package checker. The archived package retains its own source and
17-check receipt.

The current 42-phase qualification records `--bounded-workspace`: completed
test executables are retired between targets while every installed-host and
continuation phase and the original phase budgets remain required. Omit the
flag for Cargo's original aggregate workspace command. Historical `dc73996`
predates this option and retains its original 41-phase commands and results.
Source-only mode remains an explicitly partial host qualification even though
it uses the bounded workspace strategy by default.

The Gate 4 offline runner is bounded and account-free. Raw logs and machine-readable
results stay under ignored `.local/` directories. The package checker verifies
archive paths and hashes, compiles the extracted native SDK, runs installed
native/HTTP language clients, checks restart and shutdown, and exercises
backup-backed upgrade/restore without touching an existing installation.

## Recorded Gate 3 review

The [qualification report](evidence/gate-3/acceptance.html) records the clean
source candidate, all 27 offline phases, 10 installation checks and separate
live audio/context observations. It preserves unsuccessful trials and the
local topology, context and transcription limits. The package is a local
review candidate; full v0.1 and community publication remain later work.

## Pre-Gate 4 follow-up candidate

The [repair review](connect-pre-gate-4-review.html#closure) records later source
changes and their new offline evidence. The original Gate 3 installed-package
and live-media proofs above remain historical; they do not qualify an updated
archive automatically. Check the candidate directory’s `QUALIFICATION.json` for
its installation result; the source repair report may predate packaging.

SMS routes additionally require an authenticated, fresh enrollment authority.
Configure `CONNECT_SMS_AUTHORITY_URL` and `CONNECT_SMS_AUTHORITY_TOKEN` privately
on the provider-enabled server. An approved campaign or configured route alone
cannot authorize a message. See [SMS eligibility](connect-sms-eligibility.html)
for reviewed task activation, the 90-second freshness limit, keyword handling,
and the completed live suppression/recovery evidence. Database migrations are additive
and still require the explicit verified-backup procedure above.

## Public pre-Gate 4 entry qualification

A Git checkout with full history can build the actual historical schema-6 binary
without a privately supplied executable or ignored fixture script:

```sh
bash scripts/prepare-dependencies.sh
python3 scripts/build-recovery-baseline.py
python3 scripts/check-schema-six.py --previous-binary .local/recovery-baselines/8bf5bd31a519a5fa84ef8f80e4da3b58374c6de5/connect
python3 scripts/check-gate-3.py
```

The baseline builder exports exactly commit `8bf5bd31a519a5fa84ef8f80e4da3b58374c6de5`,
prepares its pinned dependencies and builds with its lockfile. Its private manifest
records the source tree and executable digest. The recovery check validates the
manifest and actual database schema, creates history with that old executable,
checks retained rows and receipts after upgrade, proves the old executable refuses
the newer schema, and restores a matching backup. An injected schema-7 table
conflict must roll back to schema 6. No existing installation is used.

The standalone schema-6 recovery check can use a separately built,
manifest-verified baseline supplied with `--previous-binary`. The complete Gate 4
source runner also needs schema-6/schema-10 Git objects and therefore uses a
full-history checkout. A source archive cannot synthesize them; do not call the
complete source run a pass because archive/installed checks passed.

Both pull requests and main pushes run the named Gate 3 offline lane and schema-6
recovery in CI, in addition to source checks. The lane includes the explicitly
ignored 1/4-session pressure profile and both SIP and WebRTC allocated audio-queue
budget tests. Account-dependent rehearsals never run in CI. Installed archives
still require `check-package.py` as separate release qualification; no new installed
archive is implied by a source check.

## Operational readiness

Authenticated `health` responses retain installation/schema/profile and add
`readiness`: an aggregate plus named component observations with stable reasons.
Worker-running means a local consumer is alive; provider-observed means the last
attempt succeeded. Neither promises that a future external request will succeed.
SIP readiness uses the registrar-accepted expiry, not only its coarse state flag.
Pending refresh can be degraded; failed/expired registration is unavailable;
unobservable state is uncertain. No registrar configured does not imply a valid
registration. Inspect route configuration separately through `capabilities`.

Fatal worker exit/panic wakes the CLI supervisor and drains the server. It never
retries uncertain effects. Embedded runtimes can await `service.readiness.failed()`
and own their shutdown policy. Registration loss changes readiness without
replaying calls; rvoip owns transport refresh/retry. A rejected registration may
require a controlled restart once configuration/carrier policy is corrected.

The prior live long-idle telephone failure is retained as an unreproduced carrier/
network observation. Local accelerated-lease tests now cover idle refresh,
rejection, dropped responses, expiry and recovery, but do not establish long-term
carrier/NAT reachability. Always-reachable telephone service is not advertised by
this local qualification; a later live soak must be separately bounded and recorded.
