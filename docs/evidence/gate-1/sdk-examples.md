# M4 — native SDK and independent app / agent journey

- `cargo test --locked -p connect-server --test sdk_facade`: 2 passed.
  All typed convenience methods, accepted/rejected/not-found receipts, exact
  create/send replay, history after close, explicit cancellation and request
  deadlines. Cancelled/timed-out peers close; a reopened client recovers the
  same receipt/cursor without hidden retries or replacement logical IDs.
- `cargo test --locked -p connect-client --doc`: 1 compiling documentation test
  passed. Runnable commands and SDK semantics are in `docs/SDK.md` and
  `examples/review-loop/README.md`.
- `python3 scripts/check-examples.py`: exit 0. Three independent processes:
  Connect server, review app and deterministic agent. Both clients initiate
  distinct conversations and reply using only the public native SDK. Offline
  draft-003 reaches the restarted agent. Rewinding a disposable app cursor does
  not repeat its local business action. Server/client restarts retain two-way
  history, saved command IDs, results, pending replies and work checkpoints.
  The actual `connect demo verify --config ...` command also passes, launching
  separate workers, inspecting app-owned state and checking native history.
- `python3 scripts/check-example-recovery.py`: exit 0. Abrupt app exits before
  effect commit, after effect commit and after server acknowledgment before
  local receipt storage all recover. Each app action remains once and its reply
  outbox resumes; another restart remains stable. Normal example binary has no
  `CONNECT_EXAMPLE_FAULT_STAGE` control.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
  Intentional assertion unwraps are allowed only in test fixtures; production
  code remains checked. All source formatting uses the scoped format script.

Crate/module boundaries: protocol, SQLite service state, transport-independent
service, SDK configuration/native exchange/errors/operations, CLI composition,
and independent review-loop work store/worker. The examples do not depend on
connect-store or read the Connect server database. Their own SQLite action is
the deduplicated business effect; external side effects require application
idempotency and are not claimed exactly once by this sample.

The agent is a deterministic responder, not Vapi/LLM behavior. This closes the
SDK/example gate. M5 still must build and install a local candidate, perform the
packaged walkthrough/recovery drills, and sign off every acceptance item.
