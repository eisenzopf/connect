# Connect contributor conventions

- The implementation contract is `docs/connect-gate-1-implementation.html`.
  Preserve stable IDs and update progress with real verification evidence.
- Keep protocol, store, service, client and server responsibilities in their
  workspace crates. Examples communicate through the public client interface.
- Run `scripts/prepare-dependencies.sh` before building. Record upstream edits
  as patches plus source-lock digests, never as undocumented `.deps` changes.
- Use `scripts/format.sh`; `cargo fmt --all` includes upstream path dependencies.
- Keep credentials, private state, raw logs and release scratch under ignored
  `.local/`, generated upstream sources under `.deps/`, and packages under `dist/`.
- This plan qualifies local macOS arm64 only. Do not add remote deployment,
  public ingress or live provider operations to its checks.
- The journal is authoritative, notifications are hints. Preserve logical
  mutation IDs across uncertainty and enforce current authorization on replay.
- Each gate closes with behavior evidence, including failure/recovery cases.
  Record skipped or unresolved checks explicitly rather than calling them passes.
