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
