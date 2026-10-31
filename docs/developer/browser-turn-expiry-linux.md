# Owned Linux browser TURN expiry

G6-054 requires actual credential-expiry refusal and resource recovery. The Linux
browser workflow now includes the two existing ignored expired-credential tests,
with both UDP and TCP coturn control. The `expired-turn` lane runs these four
independent cases; the full lane also requires them. The original Rust and browser
test assertions, eight-second browser offer budget and original Node deadline are
unchanged. The outer case budget is 120 seconds.

Each case owns a fresh network-disabled browser namespace. The exact pinned
coturn 4.18.0 image shares only that namespace, with loopback listeners and an
explicit sixteen-port relay range. Synthetic REST identities expire before the
case begins. No host ports, external service, provider, account or model are used.
The original test must refuse before a logical offer or physical allocation and
close its original contexts, peer and capture tracks. A real listener probe
matches its STUN transaction before the test. Actual 401 and expiry-specific
coturn log observations are required; zero successful allocations alone cannot
pass. Coturn must report zero allocations before its original process exits zero.
The original browser children, software-audio process and namespace holder must
join, and both containers must be removed without forced cleanup.

Container name, random ownership label, exact image and network mode are saved
before creation. An uncertain create reply can recover only the original owned
container; mismatched identities refuse cleanup. Tests cover that recovery,
foreign-resource refusal, forced-cleanup reporting and rejection of missing
authentication refusal, successful allocation, logical offer or retained capture.

The run binds the original producer binary, prepared source manifest, browser
assets, locked automation sources, browser and coturn images and tool hashes.
Only numeric and fixed-enum summaries and input hashes are uploaded. Synthetic
credentials, private database, raw browser/test/coturn logs and custody records
stay under ignored `.local/gate-6/turn-expiry-linux/`.

This lane qualifies expiry refusal only. Distinct-network relay media and the
unchanged decoded audio/video quality requirements retain their own cases. The
Linux execution of this new lane is pending; local boundary tests and syntax
checks are implementation evidence, not a runtime pass.

The initial holder at `d9bca15e` used one `signal.pause()` call and exited when
an orphan's child-exit signal interrupted it. A direct local process comparison
reproduces that exit with the exact original code. The corrected holder keeps
waiting after child-exit signals, reaps original exited children and exits zero
on its owned termination signal. The fifth boundary test executes this lifecycle
and joins its original process. The pending first Linux dispatch was cancelled
before start so the corrected holder can be verified in the same four-case lane.
This process-only comparison is not a Docker or browser runtime pass.

The first Linux execution (`41204143024`, source `d2ef6a75`) stopped at the
coturn version preflight with exit255, before any expired-browser case. Its
producer/source inputs verified and its cached runtime build completed in0.365s.
The runner restores the existing coturn helper's `NET_BIND_SERVICE` capability
for the version probe and coturn sidecar executable. The browser holder retains
no added capabilities, and both fixtures keep their original isolated namespace.
No case pass is inferred from the successful cached build.
