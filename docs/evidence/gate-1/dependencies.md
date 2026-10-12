# G1-003 — Reproducible source graph

2026-10-07, macOS arm64, Rust 1.97.1.

- `bash scripts/prepare-dependencies.sh --cache /tmp/connect-source-cache`: exit 0.
- Repeated `bash scripts/prepare-dependencies.sh`: exit 0; revisions, URLs,
  patched-file hashes and exact change inventory matched.
- Reproduction in a fresh temporary directory containing only the tracked
  scripts, patches and source lock: exit 0. The optional cache provides Git
  objects from the audited clean snapshots; the clones verify pinned HEAD and
  restore the recorded origin URL. Default preparation fetches the same exact
  revisions from GitHub.
- Appending to the recorded QUIC patch in that disposable copy caused
  preparation to fail before applying changes, as required.
- Existing `/Volumes/D2-2019/Developer` upstream checkouts were not modified.

Authoritative source and patch identities: `dependencies/source-lock.json`.
The workspace builds from `.deps/` and the checked-in Cargo.lock. That directory
is generated; it contains no Connect-owned source or credentials.
