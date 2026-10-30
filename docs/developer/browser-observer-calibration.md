# Browser observer calibration

G6-054 retains a 24 fps minimum for independently observed changing video.
An RTP decoder counter cannot replace the pixel observation. Before attributing
low observed frame rate to a transport, isolate the renderer with the same
640×360 synthetic motion, identity cells, white AV cadence and 8×8 pixel
comparison used by `scripts/video/browser-qualify.mjs`.

`scripts/video/check-renderer.mjs` runs two fresh contexts for eight seconds.
It records source draws, video callbacks, changing pixels and animation
callbacks separately. Both observers must be visible and reach 24 callback
and changing frames per second. This is a short renderer check, not a
60-second operating profile, RTC/Core/audio proof or physical display claim.

Install the integrity-locked private fixture tool with
`PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm ci --prefix scripts/video/observer-tools --ignore-scripts --no-audit --no-fund`.
Set absolute `CONNECT_CHROME_BINARY` and `CONNECT_PLAYWRIGHT_MODULE` paths;
on macOS also set `CONNECT_CHROME_FRAMEWORK` to the installed framework binary
to bind the engine rather than only its launcher. Run
`node scripts/video/check-renderer.mjs .local/gate-6/renderer/new-run`.
The output directory must be new. Source/tool hashes, numeric measurements
and original process shutdown are recorded. Profiles remain private.

The launcher matches the canonical Rust fixture's headless/CDP flags and owns
its new profile and process. Linux root containers additionally require
`--no-sandbox`; they contain no credentials and have networking disabled.
Forced process termination is a failed observation.

The separate `Gate 6 Linux browser observer` workflow uses the existing pinned
Playwright image on Linux x86_64. It requires exactly one Chromium executable
in that image's `/ms-playwright` cache, records the actual CPU-specific path and
hashes the executable; it never downloads or selects another browser. It
returns ownership of only its fixture output to the CI runner for upload while
keeping private profile permissions. It exposes no host browser or sound device,
uses no RTC/server/account endpoint and uploads only numeric/source manifests.
Its result qualifies only the measured renderer. Normal TLS, decoded media,
ICE/recovery, relay, loss, expiry and full Gate 6 acceptance retain their own
required evidence.
