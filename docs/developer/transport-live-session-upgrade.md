# Live Session upgrade in the native transport fixture

The media component now exercises capability addition on the original accepted
Sessions while four original native data Connections remain active. Two peers
in each Session exchange distinct addressed binary payloads before and after
adding audio/video capability. The fixture checks the same Connection IDs and
generations, the original Session IDs, current unoffered-screen refusal and an
unchanged snapshot after each refusal. All operation and message IDs are saved
in the parent's original private plan before the first public SDK request.

The four data Connections must then retire under their original end operations
before the existing six-Connection Opus/VP8 workload opens. End receipts alone
do not release this stage: each original Connection needs a newer retired
generation within the unchanged five-second end budget. The subsequent media
workload uses the same two upgraded Sessions. Its original six-route independent
decode, source identity, quality and teardown checks remain required.

Four Node boundary tests cover both data rounds, changed original generations,
changed received payloads and current-authority refusal without resubmission.
The independent evidence verifier rejects missing post-upgrade packets, repeated
payload digests, missing original retirement and incomplete Session transitions.
These controlled boundary tests do not prove actual native execution. The new
version-2 media producer still requires its own complete source-bound host result.

The actual new stage separately passes on local macOS arm64 with Node22.16.0,
EdDSA and the independently verified 10abb934 production runtime. The isolated
probe checks all eight received payloads, unchanged original Connection IDs and
generations, both capability transitions/refusals, all four original Connection
retirements, both Session ends and both helper/server joins. Its post-run audit
matches197 SDK producer inputs to Git and192 actual SDK files to the verified
archive. This probe has no packet capture or AV decoding and did not capture a
full SDK before/after file guard. The original qualified fixture remains byte
unchanged. The initial probe failed before the upgrade scene because process counting
included its own ps scanner; that result remains private. See the
[local stage result](../evidence/gate-6/native-live-data-session-upgrade-local.json).

This stage does not establish continuity of preexisting audio, physical Device
replacement, paired AV synchronization, any complete G6-058 cell or Gate 6
acceptance. It makes the data-to-AV upgrade observable through the public SDK;
the remaining obligations keep their existing requirements.
