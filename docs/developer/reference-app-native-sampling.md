# Reference app native server observation

G6-072's installed app has retained failures in both CI host profiles. The
bounded HTTP reuse change passes its local correctness cases, but Linux run
41299837239 still fails combined browser media. A later signed supervision
timeout records 170 ms event-loop active, 1830 ms idle and 175 ms process CPU
within its original two-second deadline. This warrants observing the native
server while preserving the actual media and control criteria.

The `reference-app-components` lane accepts `native_server_sample=true`.
On macOS only, this adds `--sample-native-server` to the installed app checker.
It starts immediately before the original two synthetic microphone actions:

- Select exactly one `connect serve --config …` process from the original owned
  application process group. Match the exact private runtime and configuration
  paths and verify the runtime SHA-256 against the installation proof. Refuse
  absent, duplicate, foreign-group or mismatched targets.
- Run `/usr/bin/sample` for eight seconds at a 10 ms interval. The observer has
  a 12 second hard bound and is joined before the original server stops. Only
  an overdue observer may be force-killed; the sampler never restarts or
  terminates its server target.
- Read process CPU and RSS before and immediately after the original observer,
  then recheck the exact target identity. These are cumulative process counters;
  wall time and inclusive stack counts are separate quantities.
- Keep raw stacks in an exclusive private `.local` file, capped at 8 MiB. Feed
  only bounded Rust symbol tokens to the platform `c++filt`, with a separate
  three-second subprocess bound. Publish only a digest and bounded known
  Rust/SQLite/runtime-wait symbol counts. Paths, addresses, PIDs, command lines
  and unknown symbols are excluded from the curated artifact.

A requested but unavailable observation is explicitly unsuccessful. The Linux
job retains the full app checks and optional Node CPU profile without trying to
run a macOS observer. All 37 app checks, media quality criteria, authorization
checks and SDK deadlines remain in force. Sampling overhead may affect timing;
an observed stack is diagnostic evidence, not qualification or causal proof.

The local calibration uses the existing source39 production macOS runtime and
owned synthetic server state. It proves observer execution, Rust demangling,
private file handling and original observer/server joins, with no media quality
or current CI runtime claim. Its first controller invocation failed to import
the sampler because of a relative path error; the original server still joined.
Corrected observations and that failure are retained under ignored `.local`.

The preceding Mac run stopped before the app because the synthetic Python
helper's cold interpreter startup exhausted its first packet-test deadline.
The fixture now waits for its control readiness before the packet cases. This
setup-only budget is five seconds; packet operations retain their one-second
budget. The one-millisecond attachment ownership test now holds and releases
the original attachment explicitly, so it tests outstanding ownership rather
than relying on interpreter startup latency. All 131 Node helper/declaration
checks pass locally on Node 22.16.0. Actual after-fix CI remains required.
