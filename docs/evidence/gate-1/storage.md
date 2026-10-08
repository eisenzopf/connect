# M2 — transactional messaging and local recovery

Environment/source graph: `environment.md`, `dependencies/source-lock.json`.
Actual schema: 2. Commands/results:

- `cargo test --locked -p connect-protocol -p connect-store -p connect-service
  -p connect-server --tests`: passed. Durable request/effect/event transactions,
  eight concurrent identical retries, changed-input conflicts, rejected and
  accepted result lookup, close/history semantics, current receipt authority,
  signed identity/resource cursors, UTF-8/page/participant limits and normalized
  membership. Eight competing sends cannot overshoot the journal row cap.
- `cargo test -p connect-store -p connect-service`: passed after the additional
  boundary cases. Actual SQLite FULL induced by max_page_count rolls back the
  attempted message, receipt and event. The service test deterministically
  commits between finished empty catch-up and the beginning of the wait; the
  event is recovered within 1 second rather than the 5-second wait deadline.
  Empty waits are bounded; physical-peer cancellation stops inbox waiting.
- `python3 scripts/check-crash-replay.py`: exit 0, all three independent process
  faults passed: before commit, after commit and before acknowledgment. The
  request client receives no success. Restart shows not_found before commit or
  accepted after commit; exact retries return the original receipt, different
  content conflicts, one message and one message event per recipient remain.
  The normal binary was checked to exclude the fault-control string.
- `python3 scripts/check-recovery.py`: exit 0. Online backup, fresh-directory
  restore on another loopback port, receipt/inbox/cursor preservation, newer
  cursor rejection, checksum tamper rejection, protected operator-only backup,
  duplicate-instance refusal, CLI credential issue/revoke, actual prior binary
  upgrade/refusal/rollback, intentionally failed upgrade and snapshot recovery.
  Original state and its post-backup accepted request remain intact.

Limits include an explicit operation-receipt count (100,000 default). Saved
replays still recover at capacity; new requests have no effects or acceptance
receipt. Journal/message limits are enforced inside serialized admission.
Storage failures propagate rather than being treated as filtered inbox events.

Recovery semantics and earlier artifact provenance: decision 004. No remote
services, carriers or providers were used. This closes M2 storage behavior;
M3 transport parity and M4 public SDK/examples remain separate qualifications.
