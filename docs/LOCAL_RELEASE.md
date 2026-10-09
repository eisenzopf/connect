# Run the Gate 3 local candidate

This candidate packages the Connect server, native Rust helper, Python and
TypeScript SDKs, browser facade, and the human conversation test client for
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

The candidate migrates supported schemas 1–8 to schema 9. A live Session becomes
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
bash scripts/check-gate-3.sh
bash scripts/check-local.sh --previous-binary /absolute/path/qualification/connect-schema1
python3 scripts/check-package.py /absolute/path/to/candidate-directory
```

Gate 3's runner is bounded and account-free. Raw logs and machine-readable
results stay under ignored `.local/` directories. The package checker verifies
archive paths and hashes, compiles the extracted native SDK, runs installed
native/HTTP language clients, checks restart and shutdown, and exercises
backup-backed upgrade/restore without touching an existing installation.
