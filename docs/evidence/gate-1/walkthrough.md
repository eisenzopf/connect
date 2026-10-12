# M5 — fresh installed walkthrough

The isolated package check used only extracted runtime binaries, provided
instructions and generated state for initialize/serve/health/diagnostics,
separate review app and deterministic agent, demo verification, restarts and
SIGTERM shutdown. No source checkout, compiler or `.deps` is needed to run the
server/examples. Separately, the extracted full source archive built the SDK.

A retained second local walkthrough verified `SHA256SUMS`, extracted the runtime
into `.local/releases/connect-0.1.0-alpha.1-macos-arm64-263fe1592ceb`, then used the installed binaries:

- Fresh state: `.local/state/gate1-review-263fe1592ceb`.
- Two-way `demo verify` passed; normal client work files were retained.
- Agent was offline when draft-004 was accepted. Its restarted worker recovered
  the request and performed its second action. Server restart and another demo
  did not repeat that action.
- Online consistent backup: `.local/backups/gate1-review-263fe1592ceb`.
- Isolated alternate-port restore: `.local/restore-test/gate1-review-263fe1592ceb`.
  The restored conversation retained its two messages and accepted send receipt.
- All servers were shut down; no background listener was retained.

Commands are in `docs/LOCAL_RELEASE.md`. Substitute the installed absolute binary
paths for its CONNECT/EXAMPLE labels. No undocumented manual workaround was
needed. Historical state and unrelated local work were preserved. The retained
installation and backups contain protected credentials and are not committed.
