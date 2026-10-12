# G1-001 — Supported local build environment

Measured on 2026-10-07:

- macOS 26.6.2, build 25G83; `uname -m`: arm64.
- Explicit project toolchain: Rust/Cargo 1.97.1 (`rustc 1.97.1 (8bab26f4f 2026-07-14)`).
- Node v26.7.0 for the TypeScript/helper experiment.
- Python 3, Git and Apple command line developer tools are needed for source preparation and builds.
- SQLite is bundled through rusqlite. QUIC/TLS uses Quinn with the ring Rustls provider.

Reproduction: install the named toolchain with rustup, then run
`bash scripts/prepare-dependencies.sh` before Cargo commands. The project toolchain
file selects this version without changing the global Rust default.

Only macOS arm64 is the initial installation qualification target. Other
platforms are not qualified by the tests on this host.
