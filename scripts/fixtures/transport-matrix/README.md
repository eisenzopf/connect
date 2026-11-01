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
