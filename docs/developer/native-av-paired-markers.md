# Native six-Connection paired markers

G6-058's native media producer uses `connect/native-av-sources/2`. The source
profile is frozen in [decision 015](../decisions/015-gate-6-operating-profile.md).
It preserves six distinct speaker tones, six source-color corners, moving
640×360 VP8 at30fps, mono48kHz Opus at32kbit/s,62seconds of source and the
original60second observation window after two seconds of warmup.

Each source adds the existing `connect-av-cadence/1` nonperiodic programme from
`scripts/video/marker_calibration.py`. Each100ms pulse raises the tone's peak
amplitude from0.05 to0.7 and changes a separate80×80 patch at96,0 from black to
white. The source-color corner remains visible during a pulse. Frame indices
define both original gain and pixel transitions; no sender report assigns an
observed marker's ordinal.

The independent FFmpeg decoder retains its original speaker, source-pixel,
moving-frame, decoded-audio duration and original RTP progression checks. It
also decodes the pulse patch and the original20ms audio packets. Pixel/gain
levels must prove an onset after200ms of continuously observed low content.
Initially high levels, a short low rebound and a missing observation interval
cannot establish a new onset. Audio uses the existing0.045/0.055 RMS hysteresis
and50ms observation-gap bound; video uses black/white thresholds16/240 and a
100ms observation-gap bound. These observer limits do not extend the media's
original200ms freshness or presentation allowances.

The shared bounded cadence calibrator assigns ordinals independently to the
decoded audio and video edges. It pairs equal ordinals only. Ambiguous or missing
calibration remains a failure; nearest periodic edges, RTP origins and wall-clock
offsets cannot repair the observation. Every source2 route must pass at least42
pairs,80ms median absolute skew and120ms p95 over the60second window, in addition
to all existing quality checks. Omitting or changing the frozen source2 marker
programme is refused before reading the media observations.

This component correlates decoded marker content with the original packet/frame
receive times on one caller monotonic clock. It does not measure live decoder
completion or physical speaker/display presentation. Independent numeric sender
reports and local queue receipts remain separate diagnostics. A direct codec
roundtrip calibration uses generated sample/frame coordinates and cannot prove
transport, receiver timing or a complete matrix cell. Actual six-Connection
execution, transitions, impairment, privacy, fallback and owned drains remain
required. Earlier source1 failures retain their original results.
