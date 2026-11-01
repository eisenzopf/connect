# Incoming media during local camera startup

The reference app starts negotiated incoming reads after device permission and
track validation, before waiting for local worklet and camera preview setup.
Receiving from an already consented Session does not depend on the local camera
preview becoming ready. A slow preview must not leave incoming audio queued
behind that unrelated setup.

A controlled test holds `camera.play()` unresolved while the real
`BrowserAppMedia` class starts. Before the change no receive opens; afterward the
original receive is pending while preview is still held. Stopping at that point
aborts and joins the original read, closes the admitted lease/codecs/context and
stops both local tracks. A late preview result schedules no camera callbacks.
Denied device permission still starts no incoming reads.

Diagnostics record device setup, receive startup, worklet graph and camera setup
durations. These are elapsed times only. RTP/NTP source timestamps, current
SDK/native authorization, original operation IDs and all playout/send deadlines
and queue limits are unchanged. No delayed frame is relabelled as fresh.

The source8a Linux rehearsal still fails after25checks: most decoded audio is
late while video remains much nearer its target time. No supervision timeout is
recorded in that run. The held-preview counterexample establishes a startup
coupling; it does not prove that coupling caused the Linux failure. New installed
rehearsal results remain required. See
`docs/evidence/gate-6/browser-duplex-startup.json` and
`docs/evidence/gate-6/reference-app-source-paired-sql-linux-negative-8a.json`.
