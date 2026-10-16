# Connect contributor conventions

- Implementation contracts are `docs/connect-gate-1-implementation.html` and
  `docs/connect-gate-2-implementation.html` and
  `docs/connect-gate-3-implementation.html`; their product plans define scope.
  Gate 4 approved plans are `docs/connect-gate-4-plan.html` and
  `docs/connect-gate-4-implementation.html`; the owner authorized implementation.
  Gate 5 plans are `docs/connect-gate-5-plan.html` and
  `docs/connect-gate-5-implementation.html`; the owner authorized implementation
  and parallel agents on October 13, 2026. Publication remains a separate action.
  Gate 6 approved plans are `docs/connect-gate-6-plan.html` and
  `docs/connect-gate-6-implementation.html`; the owner approved both plans and
  authorized implementation on October 15, 2026. Follow their dependency graph.
  Breaking pre-alpha API, SDK and schema changes are allowed; do not add legacy
  API compatibility or downgrade work. Server HTTP/OpenAPI reference is required.
  Preserve stable IDs and update progress with real verification evidence.
- Keep protocol, store, service, client and server responsibilities in their
  workspace crates. Examples communicate through the public client interface.
- Run `scripts/prepare-dependencies.sh` before building. Record upstream edits
  as patches plus source-lock digests, never as undocumented `.deps` changes.
- Use `scripts/format.sh`; `cargo fmt --all` includes upstream path dependencies.
- Keep credentials, private state, raw logs and release scratch under ignored
  `.local/`, generated upstream sources under `.deps/`, and packages under `dist/`.
- Gates 1–5 qualify their recorded local macOS arm64 scopes. Gate 6 additionally
  targets the approved Linux x86_64 hosting and remote/native/browser profiles.
  Default checks use isolated state and fixtures. Named remote/provider rehearsals
  require the concrete bounded configuration in G6-093 and designated test
  endpoints; keep those lanes explicit. Publication remains separate.
- The journal is authoritative, notifications are hints. Preserve logical
  mutation IDs across uncertainty and enforce current authorization on replay.
- Each gate closes with behavior evidence, including failure/recovery cases.
  Record skipped or unresolved checks explicitly rather than calling them passes.
