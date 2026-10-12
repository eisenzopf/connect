# M5 — default diagnostics and archive hygiene

Package qualification exited 0. It verified all archive SHA-256 values and every
runtime manifest entry, then rejected links, special files, escaping paths,
generated/private directories, DER keys/certificates, DBs and raw logs in the
archives. Runtime binary byte scans found neither test fault control. The
source archive contains feature-guarded test source; it contains no fault binary.

The fresh installation's files have no group/world permission bits. Default
server diagnostics retain operation/status and SHA-256 fingerprints of exchange,
logical request and credential IDs. Raw tokens, private keys, canary message
body and raw canary logical request ID were absent. Deliberate API response
stdout is distinguished from diagnostics; request bodies are useful API data.

The first checker run inherited this host's `RUST_LOG=warn`, so it correctly
failed its expected correlation assertion. The checker now removes that override
only in its own process environment and strips ANSI formatting. A repeat passed
with actual default operation correlation and redaction. No server logging
behavior was weakened to make the test pass.

The runtime includes project/upstream license texts and registry license/notice
files with a full-workspace dependency index. Keys/configs/work files/backups
stay under ignored `.local/`; dependencies under `.deps/`; builds under `target/`;
unpublished archives under `dist/`. No Rudeless website, remote service, system
PATH, daemon, DNS or tunnel was changed.
