# Canonical authentication recovery boundaries

This guide records the authentication recovery increment. It addresses peer-lifetime replay,
signature problem classification and completion of failed step-up exchanges.
It does not close the remaining reachability, transport installation or final
normative-clause audit.

`CanonicalPeer` now owns a non-evicting replay fence for its entire physical
lifetime. Every successful client signature verification consumes its envelope
ID before any waiter or application handler admission. Auth proofs, refresh,
keepalive, logout, controls, correlated replies and step-up responses share the
same namespace. Fresh timestamps/signatures do not make an old ID reusable.
Refresh and step-up install a new dispatch incarnation with the same fence.
The private held-request proof resumes its already consumed invocation once;
it never consumes the ID a second time or exposes a public replay exemption.

This fence supplements the verifier's atomic replay store and five-minute
freshness checks. Neither is disabled. Default capacity is 4,096 bounded IDs;
`with_replay_capacity` permits a smaller explicit capacity. The challenge
advertises the effective bound and `replay_usage` provides counts without IDs.
Capacity exhaustion fails closed and never evicts an old ID. Invalid signatures
do not consume fence slots. Reconnect, restart and multi-process logical Session
retention require their durable replay/operation policies; a physical-peer fence
does not qualify those lifetimes.

The new clock-controlled tests exercise the actual verifier/store and canonical
runtime. After the real external replay entry expires, the store records a second
successful cryptographic admission of a newly signed reused ID. The peer fence
still rejects it. Separate cases cover refresh, step-up, auth/control namespace
sharing, capacity pressure and a fresh envelope recovering the same logical
operation. The fixture's one-effect set is process-local, not a durable ledger.

Version 2 exposes these bounded signature problems, all in category `auth`:

| Code | Reason | Meaning |
|---|---|---|
| 401 | `signature_required` | A required signature is absent. |
| 401 | `stale_timestamp` | The signed timestamp is outside the allowed window. |
| 401 | `invalid_signature` | Signature/key/algorithm verification failed. |
| 401 | `replay_detected` | A consumed envelope ID was reused. |
| 503 | `replay_protection_unavailable` | Atomic replay storage is unavailable. |
| 429 | `replay_capacity_exhausted` | The physical peer's fence is full. |

Unknown keys and invalid cryptographic signatures produce identical problem
payloads. Credentials, key IDs and payload content are never echoed. The v2
machine reasons are explicit contract spellings; no v1 problem alias is added.
Real pinned-TLS/QUIC tests verify signed, correlated problems and graceful signed
logout/lifetime retirement.

A matching authenticated step-up response consumes its pending proof for one
attempt. On failed validation, the peer records only the two minimal correlation
scopes, without the original body, arbitrary extensions or supplied credential.
It emits a 403 `assurance_not_established` result for the original mutation and
a 401 `step_up_failed` result for the step-up response. Both keep the original
logical operation scope. This finishes both waiters; a fresh signed envelope
may attempt the same authorized logical operation again through the ledger.
Wrong scope or an invalid signature cannot consume the legitimate challenge.

The failure guard also runs when an asynchronous validation/execution future is
cancelled. Before handler execution, the original held mutation did not execute.
Once execution may have started, its 409 `operation_interrupted` problem carries
`details.outcome: unknown`; the response receives 408 `step_up_interrupted`.
This does not invent a failed business outcome. Reconcile under the same logical
operation ID. A fixture deliberately applies an effect before cancellation and
checks that the result remains unknown.

There is one bounded failure slot per physical peer. It cannot evict another
receipt; another challenge requires it to be drained. `run` drains it before
accepting more frames and after a failed completion, using the same bounded,
cancellation-aware writer as ordinary results. Direct `handle` integrations
must call `drain_step_up_failures` after errors or cancellation and retain the
returned output until writer admission. A slot collision retires the peer.
Terminal peer closure discards local notifications; durable operation recovery
remains the application's responsibility.

Run `python3 scripts/check-gate6-auth-recovery.py` for the source-bound local
qualification. New tests use real Ed25519/JCS and canonical runtime, including
actual `uctp/2` TLS/QUIC problem/recovery exchanges and driven validation timeout.
Credential, identity/device, resource and revocation callbacks remain controlled
fixtures. Reachability/reconnect, production QUIC/WT/WS profile installation,
Connect durable service policy, cross-peer/Session restart replay retention,
browser/SDK/provider/remote and media-quality qualification remain open. No Gate 6
acceptance is inferred.
