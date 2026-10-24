# Fifty-Participant native Session rehearsal

`scripts/rehearse-gate6-fifty-participants.py` runs the actual native QUIC
capacity and protocol case through Connect and the public Rust client. It uses
isolated local installations and generated fixture credentials. It does not
contact a configured remote endpoint, provider or PSTN destination.

The default profile is Cargo `--release`. Use `--profile test` to record the
unoptimized test profile separately. A passing optimized run cannot turn a
failed unoptimized run into a pass. Neither profile qualifies Linux, browser,
remote/provider, independent-client or whole-gate acceptance by itself.

This resource-intensive case is ignored by ordinary unit-suite runs. The
rehearsal runner explicitly includes it and rejects a zero-test or skipped
result. An ordinary suite's skip is not a capacity pass. The source178
unoptimized failure remains recorded regardless of this harness separation.

Install the capture validator in an ignored environment:

```sh
python3 -m venv .local/gate-6/fifty-validator
.local/gate-6/fifty-validator/bin/python -m pip install -r scripts/requirements-gate6.txt
```

Run the fixed workload with an appropriate local build cache:

```sh
python3 scripts/rehearse-gate6-fifty-participants.py \
  --profile release \
  --target-dir "$HOME/.local/share/connect/gate6-fifty-target" \
  --python .local/gate-6/fifty-validator/bin/python
```

Each run creates a fresh directory under `.local/gate-6/fifty-native`. An
explicit `--output` must also be a fresh directory under `.local/gate-6`.
Earlier captures and failed attempts are preserved. The runner verifies the
pinned dependency preparation before building and records source hashes,
compiler environment, Cargo profile, executable hash, commands, results and
artifact hashes in `summary.json`. Relevant source changes during the run
prevent a passing summary.

The one runtime method exercises both EdDSA and ES256. In each scene it requires:

- Fifty real signed native clients and independently owned Participants,
  including Human, AI, External and System identity kinds. These are local
  fixtures, not live external agents or providers.
- Fifty send-only publisher Connections, followed by forty-nine independent
  receive-only Connections with distinct per-physical-peer Stream handles.
  Every peer observes Session start and every other Participant joining.
- No RTP, including silence, before explicit subscription. Forty-nine signed
  subscriptions select an exact Stream, a Participant, or that Participant's
  audio kind. Their completion and ACK correlation must agree.
- Actual decoded receiver delivery and durable flow facts under the original
  shared eight-second startup bound. The subsequent media window sends for two
  seconds and drains a 300 ms tail, excluding its first 300 ms from marked-frame
  counts. It requires more than ten marked decoded frames at every intended
  receiver. Protected media retains its original 200 ms age bound.
- A later real publisher on the original Participant and a new handle. Its
  media reaches the thirty-three retained Participant selectors while the
  sixteen exact original-Stream selectors remain excluded. Source end/replay
  returns to the original forty-nine-way publication without another effect.
- Unsubscribe/replay/no-op behavior, Session leave and membership removal,
  exact signed leave multicasts, current-authority refusal and continued audio
  to the surviving recipients. The actual runtime peaks at one hundred
  Connections and ends with ninety-five.

The runtime appends an observation only after an entire signing scene passes.
The SDK verifies received signatures. The separate Python validator checks the
captured envelope schemas, correlations, selector sets, handles, decoded counts
and lifecycle totals; it is not an independent cryptographic implementation.
An empty or partial capture is not capacity qualification. The runner must
execute exactly one runtime method successfully and validate both complete
scenes before `summary.json` can report `passed: true`.

Current source178 verification and its failed default-profile attempt are
recorded in the
[same-packet custody report](../evidence/gate-6/auxiliary-same-packet-source-custody.json).
That report distinguishes Store/Core component proofs, actual production
regressions and capacity evidence. Gate and clause progress must follow the
actual recorded results, not compilation or this procedure alone.
