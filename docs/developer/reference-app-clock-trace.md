# Original app media clock observations

The installed-app checker accepts `--trace-media-clock`. The component workflow
exposes the same opt-in `app_clock_trace` input. All original 37 behavior checks,
source clocks, media ages, packet bounds and playback limits remain unchanged.

The fixture observes successful original `/api/media/open`, `send` and `receive`
requests/responses. It records RTP header numbers, relative sender-report times
and HTTP durations without sending another request or evaluating browser code.
Lease, stream and SSRC identities become bounded local slots; payloads, raw
base64, CNAMEs, absolute NTP values and original identifiers are excluded.

At most four response reads are pending. Each page has at most sixteen routes;
each direction retains its first sixteen/latest thirty-two RTP observations and
first eight/latest sixteen clock observations. Response bodies are bounded to
64 KiB. Closing the fixture detaches listeners and joins existing reads under a
separate two-second observation bound. Drops, malformed observations and pending
reads remain explicit in the result.

The report replays the unchanged application `MediaTimeline` and `playoutTime`
on retained receive-event times. It reports the estimated source age and output
offset at the fixture's network observation, together with retained HTTP timing
distributions. These times differ from the browser's actual decode and playback
callbacks. Comparing them with existing browser diagnostics helps distinguish
delay already present at reception from later browser work. This is diagnostic
evidence; it does not correct a clock or qualify media quality/presentation.

Five controlled tests cover payload/identity exclusion, bounded rings, pending
read ownership, wrong origins/malformed input, and a known packet already late
before decoding. Actual hosted traces still need their own source-bound result.
