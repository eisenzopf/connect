# Gate 6 transport matrix

[The matrix contract](../../../contracts/gate6-transport-matrix.json) preserves
32 required cells: two server platforms, Local and isolated Remote-profile
topologies, native QUIC/Chrome WT/Chrome WSS-WebRTC/Firefox WSS-WebRTC, and both
signing cohorts. [The operating profile](../../../docs/decisions/015-gate-6-operating-profile.md)
and media budgets remain binding. Named remote hosts use the separate concrete
G6-093/G6-095 execution lane. G6-054 remains deferred.

Each complete cell needs its own exact candidate, client/server versions, actual
topology, current authorization/refusals, participant changes, Session upgrade,
device replacement, four-Participant/six-Connection/two-Session workload, private
packet capture, independently decoded duplex observations, media budgets and
original lifetime drain. Browser cells also require observed transport choice,
WT-unavailable fallback and neither-path-usable refusal. No old component report
or neighboring cell supplies those results.

Generate an unqualified inventory in a new ignored directory:

```sh
python3 scripts/check-gate-6-transports.py --phase inventory
```

The input templates [Local](local.json) and [Remote-profile](remote-profile.json)
declare configuration and private assets that full cell materializers must
supply. Actual assigned addresses, trust, Device credentials and owned network
cleanup must be recorded before peers open. Full cell materialization and runtime
collection remain required. The templates contain no generated credentials or
assumed accessible remote hosts.

The current non-media control component runs 15 exact existing functions against
the production listeners: independent signed HTTP3/TLS WSS, both signing
algorithms, native original-receipt parity, authenticated discovery, Origin/mount/
trust refusal, shared physical admission, retirement and actual Node browser-SDK
WSS control. Node supplies its own explicit Origin and normal TLS trust in that
fixture. It does not establish browser behavior. No media, SIP, TURN, provider,
model or named remote-host execution is selected.

With the pinned compiler, Node 22 or later and native prerequisites installed:

```sh
npm ci --ignore-scripts --no-audit --no-fund --prefix sdk/typescript
npm run build --prefix sdk/typescript
python3 scripts/check-gate-6-transports.py --phase control
```

The runner prepares verified dependencies before building. It type-checks before
optimized compilation, identifies/seals the actual test executable, verifies the
selected names exist, executes one exact function at a time, refuses zero/ignored
test passes, and rehashes source/executable/generated SDK inputs. Compiler and
runtime commands own bounded process groups. Raw logs, credentials and fixture
state stay under `.local/gate6-transports/private/`; uploaded reports contain
counts, fixed source coordinates, hashes and outcomes. Existing result directories
refuse replacement. A successful control component leaves every full matrix cell
`not-run` and `matrix_qualified: false`.

The `transport-control` lane of `gate6-linux-fixtures.yml` runs this component on
native Ubuntu24.04 x86_64/Node22. Other platforms, actual Chrome/Firefox, remote
profiles, captures and decoded media still need their explicit cell producers and
source-bound results. G6-058 stays open until every required cell verifies.

## Protected fixture configuration

`scripts/materialize-gate6-transport-fixture.py` prepares a new server/operator
instance from a **stopped canonical backup** created by the production CLI. It
invokes the same production `restore` command; it does not copy or edit SQLite
state. Keep the seed, parameters and output under ignored `.local/`, with private
directory/file permissions. Select the actual server OS, network profile and
client/server signing cohort before creating the seed. The tool refuses an
expired server pin, shared server/operator signing material, a changed provisioned
endpoint, an existing output directory or configured external components.

The private JSON parameters have exactly these fields (replace angle-bracket
values with the inspected candidate and seed values):

```json
{
  "cell_id": "G6-058/ubuntu24-x86_64/local/native-quic/EdDSA",
  "candidate_source_revision": "<40 lowercase hex characters>",
  "candidate_binary_sha256": "<64 lowercase hex characters>",
  "seed_manifest_sha256": "<64 lowercase hex characters>",
  "browser_origin": null,
  "topology": {
    "kind": "local-loopback",
    "server_platform": "ubuntu24-x86_64",
    "client_platform": "ubuntu24-x86_64",
    "client_emulated": false,
    "native_endpoint": "<exact provisioned IP:port>",
    "server_name": "<exact provisioned TLS name>"
  }
}
```

Remote-profile rows use the frozen `ubuntu-netns` or `macos-linux-guest` topology
and the matrix's expected client platform; these declarations still require
independent runtime observation. Browser rows require the exact HTTPS origin,
both `/connect` listeners with that allowed Origin, HTTPS application enrollment
and existing WebRTC configuration in the seed. A native-only seed refuses a
browser row. Actor/device enrollment, an actual owned topology, the full workload,
packet capture and media observations remain required stages.

```sh
python3 scripts/materialize-gate6-transport-fixture.py \
  --seed .local/transport-seed \
  --parameters .local/transport-parameters.json \
  --binary .local/transport-candidate/connect \
  --output .local/transport-instance
```

The output contains private `server.toml`, `operator.toml`, restored `state/`,
`restore.log` and a hash-only `materialized.json`. Only path references change;
credential, pin validity and all other configuration values are retained. No
listener is started. Materialization cannot qualify a matrix cell.

The isolated `transport-fixture` CI lane builds the normal production CLI,
creates a synthetic stopped backup, materializes it and exercises native reads of
the original conversation/operation. It also requires unchanged seed bytes,
private output modes, occupied-output/signing/browser refusal and original server
shutdown. Its result is configuration-stage evidence only; all 32 complete cells
remain unqualified. Raw logs and state stay private; only curated hashes and
outcomes are uploaded.

## Actor and Device enrollment

With the materialized production server running, enroll the matrix's four
synthetic human identities and five independent native devices. The fifth device
belongs to the first identity and is reserved for later replacement tests. Supply
the private application administrator configuration for this exact server; it
must select its loopback HTTP listener and the same operator credential. The
administrator configuration remains outside every participant installation.

```sh
python3 scripts/enroll-gate6-transport-actors.py \
  --instance .local/transport-instance \
  --admin-config .local/original-installation/operator.application.toml \
  --binary .local/transport-candidate/connect \
  --output .local/transport-instance/actors
```

The tool saves `plan.json` before public provisioning, including original identity
and credential operation IDs and an absolute expiry. All state changes use the
production `provision`, `issue` or `issue-resume` command. Each child receives 28
literal control permissions, the other three identities as its recipient ceiling,
and a new device signer matching the cell's signing cohort. It checks the returned
scope, expiry and identities, distinct credentials/keys, and absence of the parent
bearer in participant files. Credential expiry is at most one hour and never
exceeds the administrator or server pin expiry. Neither an administrator scope nor
external communications permissions are included.

An interrupted enrollment retains its files. Reinvoke the same command with
`--resume` to replay original identity operations and recover existing issuance
through `issue-resume`. The plan refuses changed candidate, issuer, scope or
expired validity. An installation interrupted before its immutable CLI intent was
saved requires inspection; the tool does not delete it or choose new IDs.
`actors.json` privately records configuration paths and identities for the cell
executor; `enrollment.json` contains curated hashes, counts and scope only.

The `transport-actors` CI lane additionally executes all five signed clients,
checks original-ID resume without asset changes, refuses a changed issuer before
network access, and verifies current revocation with an independently usable
replacement. Rehearsal revocation happens only after the enrollment checks; the
standalone tool does not revoke an enrolled device. Enrollment does not create
Conversation participants or establish actual concurrent connections, sessions,
device replacement, browser transport, media or topology outcomes. Those remain
required complete-cell execution stages.

## Conversation and Session control

The isolated `transport-conversation` lane runs the same production bootstrap,
restore and scoped enrollment, then executes
`scripts/check-transport-conversation.mjs` through the current public Node SDK.
Five real SDK helpers use the normal production binary; the fixture binds their
actual `/proc` executable hashes and requires their original exits. Raw inputs,
operation references and diagnostic logs remain private. Generated SDK files are
hashed before and after execution.

The scenario observes four active Conversation participants, add/update/remove
and readmission with a stable Participant ID and a new membership epoch, original
mutation replay, conflicting content and stale revision refusals, and message
audiences before/after membership changes. It creates and accepts two simultaneous
data Session intents, adds an offered audio capability to one intent, refuses an
unoffered video upgrade without changing that Session, and ends both intents.
After the original helper closes, an independently enrolled Device recovers the
original creation and authorized history. Logical IDs are fsynced before public
IO; no private Store or direct database access is used.

These are actual control observations. An accepted capability intent does not
prove live audio/video, a media ownership replacement or the six-Connection
workload. Actual owned topology, browser transport selection/fallback, packet
capture and decoded duplex observations still belong to the full cell executor.
The scenario's reports keep every complete matrix cell unqualified.

The Ubuntu/local/native EdDSA control stage is independently verified at
`5552d367` in [the source-bound report](../../../docs/evidence/gate-6/transport-conversation-verified.json):
13 configuration/enrollment/rehearsal checks, nine SDK scenario checks, 40 public
CLI commands, five joined SDK helpers and two joined servers. Nonempty history
reads followed their cursor through the terminal page. Earlier pagination and
active-snapshot fixture assumptions are retained as negative evidence. This
result grants no other platform, topology, signing cohort or complete cell a pass.

## Actual native data workload

`transport-data` runs the current scenario with `--data-workload`. It enrolls
fresh scoped actors with the additional literal `stream.subscribe` and
`connection.end` permissions required for owned routing/drain; an existing plan's
scope is never expanded on resume. The earlier 26-permission enrollment proof
remains bound to its original source. Current fresh plans have 28 permissions.

The trusted fixture backend first reads each safe Endpoint receipt from public CLI
issuance output. Endpoints start unverified. After observing a successful signed
native control read from each independently enrolled Device, the backend saves
the exact verification commands and proof references before public IO. It rejects
participant self-verification, verifies each Endpoint through the administrative
API and replays the original commands without another revision change. The
fixture does not inspect the installer's private intent or write Store state.

Four participants join the main data Session; two join the second Session. The
public SDK offers six distinct native Connections and subscribes each to the
previous participant's stream in its Session. All six must be ready at the common
observation boundary, carry one independently identified 64KiB binary payload to
the intended recipient, and remain current at the second boundary. Original data
replay must recover the receipt without a second delivery. Every original
Connection must retire through the public API, with owned data queues empty.

The same run observes the server's actual loopback UDP socket through its owned
socket inode and verifies that all five original SDK helpers share the server's
network namespace. Process and configuration bindings use the same production
executable and protected instance. Raw process/socket state stays private;
published topology observations contain hashes, counts and booleans.

This is a native reliable-data workload and local-topology check. It does not
qualify simultaneous audio/video, decoded duplex, live media upgrades, replacement
media ownership, packet capture, browser or remote-profile behavior. Complete
matrix cells remain unqualified until their full checks execute and verify.
