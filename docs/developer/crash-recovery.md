# Startup, crash and shutdown recovery

The journal retains logical operations, their original effects and attempts.
Restarting the canonical server obtains exclusive installation ownership and a
new runtime epoch before accepting work. It fences old workers and Connection
observations, then runs bounded recovery batches. Reopening the database cannot
restore a physical peer, decoded readiness or a live provider call.

After a possible external submission, keep the original operation ID. Read it
through `operation.get`, or replay the original command with its saved logical
ID under current authority. A retry uses a fresh transport envelope and the same
business intent. Unknown effects retain their conflicting reservations; changing
the operation ID does not authorize another dial or bridge.

The configured provider worker queries supported original-resource status through
its authenticated provider driver. It retains the original correlation, effect
attempt and receipt. A provider response can settle the fact it actually proves;
it cannot fabricate a local Session acceptance, media Connection or role commit.
A still-live SIP peer without an attributable status result stays uncertain after
host death. A local adapter's absence proves only that adapter's old resources
are absent.

For an operator decision, the public canonical `operation.reconcile` payload
names the target `operation_id`, `outcome` and an existing `evidence_id`. Supply a
separate stable decision ID in the top-level `operation_id`. Optional
Conversation, Session and Connection assertions must match the original target.
The same command is available through signed native control and
`CanonicalHttpClient` at `POST /v2/commands`; see the
[HTTP reference](../../contracts/openapi-canonical-http.json).

Current operation permission, original command/source authority, owner and
recipient ceilings still apply after restart and on replay. Evidence is bound to
the operation's current revision and worker generation. A caller cannot turn an
arbitrary evidence string or operator attestation into provider termination or a
physical commit. Native and telephone operations require their attributed runtime
settlement and refuse generic declared completion with
`reconciliation-required` (HTTP 409). Preserve uncertainty and investigate the
original effect when no supported positive proof exists.

Protected backend observation and cleanup APIs are distinct from public operator
commands. They record bounded attributed observations, recheck current authority
and cannot let a replacement operator complete a revoked owner's live transition.
Evidence admission permits at most 32 observations per operation. Recovery batch
scope is bounded and drained through repeated positive counts. Decision-source
walks reject cycles and excessive depth rather than accepting unbounded history.
These bounds do not authorize discarding unresolved effects.

Orderly shutdown stops intake, drains initializers and actual native peers,
provider and telephone workers, record and HTTP work, then retires installation
ownership. The protocol close wait has a bounded allowance after owned workers
stop. `Drop`, lost command replies and abrupt process death are separate failure
vectors in the evidence. A process kill cannot run local teardown; its successor
must report the remaining external uncertainty honestly.

The [G6-047 evidence](../evidence/gate-6/crash-recovery-scope.json) records exact
methods, source/executable guards and actual failure witnesses. Its default
fixtures are isolated on macOS arm64. They include actual server SIGKILL with
surviving external fixtures and independently decoded native audio. Controlled
Store process exits separately cover local transaction boundaries and a synthetic
effect counter. Neither scope qualifies Linux hosting or named live-provider
rehearsals. G6-047 remains under requirement audit until its recorded pending
checks are resolved.
