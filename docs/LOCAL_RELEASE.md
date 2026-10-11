# Run a local Connect candidate

This candidate packages the Connect server, native Rust helper, Python and
TypeScript SDKs, task/MCP tools, workflow skill, agent-presence example, browser
facade, and the human conversation test client for
macOS arm64. It is unpublished and performs no remote deployment. Provider-live
evidence is recorded separately from the package qualification.

## Verify and install

Verify all three archives in the candidate directory:

```sh
shasum -a 256 -c SHA256SUMS
```

Extract the runtime archive into a new `releases/` directory. Keep state outside
the extracted release. Use the absolute path of `bin/connect` below; no PATH,
daemon, DNS, tunnel, or existing installation change is required.

## Start a private local installation

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

Gate 4 package qualification is still in progress. The numbered
[implementation plan](connect-gate-4-implementation.html#G4-033) records actual
results; new packaging code is not an installed-candidate attestation.

The runtime carries [task contracts](../contracts/connect-task-v1.md),
[protected grant setup](TASK_ACCESS.md), the [workflow skill](../skills/connect/SKILL.md)
and [Codex installation/resume instructions](CODEX_TASK_HOST.md). The server
exports the matching embedded skill and private host configuration. Developer
setup grants the agent its task capabilities once; individual actions do not
require approval prompts.

The [incoming-contact example](../examples/agent-presence/README.md) runs from
`examples/agent-presence/codex-inbox.py` with `PYTHONPATH` set to the installed
`sdk/python` directory and `--executable` set to installed `bin/connect`.
Use a private application state directory and the documented explicit-contact
policy. One invocation processes one contact; later continuation uses the saved
host session. Both task history and current authority survive restart.

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
credentials. The [schema-16 local candidate](evidence/gate-4/package-candidate.json)
passes installed walkthrough, Vapi host, SDK and recovery checks. Whole Gate 4
acceptance remains open. Check the installed manifest for its exact source and schema; older Gate 3 archives use schema 10. A live Session becomes
interrupted across restart; Connect preserves durable intent and never claims a
socket or media pump survived. Roll back by restoring the matching pre-upgrade
backup into a new directory and using its compatible binary.

## Reproduce qualification

The source archive contains the full workspace, lockfile, patches, contracts,
tests, and dependency source lock. The separate qualification archive contains
the historical schema-1 milestone binary and provenance manifest, without a
database or credentials.

From the extracted source checkout:

```sh
python3 scripts/check-gate-4.py --codex-binary /absolute/path/to/codex
bash scripts/check-local.sh --previous-binary /absolute/path/qualification/connect-schema1
python3 scripts/check-package.py /absolute/path/to/candidate-directory \
  --codex-binary /absolute/path/to/codex
```

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

Source archives without Git history require a separately built, manifest-verified
baseline supplied with `--previous-binary`; they cannot synthesize historical Git
objects. Do not report that check as passed when the baseline is absent.

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
