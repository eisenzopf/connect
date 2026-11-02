# Reference app native audio diagnostics

The installed app checker accepts `--trace-native-audio` for a separate local
diagnostic run. The `reference-app-components` CI lane exposes the same opt-in
as `app_native_audio_trace`; its default is false.

```sh
node scripts/check-reference-app-installed.mjs \
  .local/gate6-reference-app/candidate \
  .local/gate6-reference-app/audio-observer \
  --trace-native-audio
```

Use a verified component installation and a fresh private output directory, as
required by the existing checker. The checker passes `--native-audio-trace bounded`
to the owned app runner. Only that runner's native server receives the fixed
`warn,rvoip_core::audio_path=trace` logging filter. Its private log is capped at
8 MiB; the ordinary default remains 2 MiB. No queue, packet, clock, authorization,
media measurement, deadline or original mutation changes.

After the original app/server group joins and flushes its logs, the checker
summarizes up to four private server logs. It returns numeric target slots,
sample counts, p50/p95/max ages, refusal/trim counters and log digests. Raw lines,
Connection IDs and paths remain private. Missing or malformed observations stay
explicit; a missing sample never becomes zero latency.

`mixer_queued_age` measures original source admission to the Core mixer output
queue. `entering_io_age` measures source admission to the guarded local IO call.
These are independent distributions, not matched per-packet transit times. They
exclude later network, SDK/backend receipt, browser decode and presentation.
Logging adds observation overhead, so this diagnostic does not replace ordinary
media verification or qualify a transport cell.

The collector's bounded-input/privacy cases pass on Node 22.16.0. A separate
signed local PCMU/VP8 AV replacement component produced 772 queue-age and 830
IO-age events, all parsed without malformed events. This establishes collector
behavior; it does not explain the Linux Opus/browser failure. See
[the scoped evidence](../evidence/gate-6/reference-app-native-audio-observer.json).
