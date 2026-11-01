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
version-2 media producer still requires its own source-bound host result.

This stage does not establish continuity of preexisting audio, physical Device
replacement, paired AV synchronization, any complete G6-058 cell or Gate 6
acceptance. It makes the data-to-AV upgrade observable through the public SDK;
the remaining obligations keep their existing requirements.
