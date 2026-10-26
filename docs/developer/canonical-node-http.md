# Node canonical HTTP transport foundation

`CanonicalHttpClient` from the Node package entry point sends finite current
canonical commands to an explicitly configured loopback HTTP origin. This is a
G6-065 consumer migration checkpoint. The old `HttpClient` still uses the older
named application interface; it is not a canonical control fallback. Full typed
Node handles, managed helper ownership and remote transport remain G6-062 work
behind its declared dependencies.

```typescript
import { CanonicalHttpClient } from "@rudeless/connect";
const client = new CanonicalHttpClient({ endpoint: "http://127.0.0.1:4434", credential });
try {
    const frames = await client.request("operation.get", {
        operation_id: savedOperationId,
    }, { cid: savedConversationId }, { timeoutMs: 5000 });
    // Inspect the current original record; an ACK alone is not a completed effect.
} finally {
    await client.close({ timeoutMs: 5000 });
}
```

Save mutation payload, logical operation ID and exact resource scope before IO.
A failed started mutation returns `CanonicalError` with `certainty: "uncertain"`
and its original key. Reconnect deliberately and query that original ID under
current authority. No request is retried, redirected or switched to another
transport automatically. A finite journal read must explicitly set `live:false`.
The transport validates batch structure and correlation; typed record decoding
and the complete raw-envelope schema contract remain above this foundation.

The request deadline includes headers, body reading and parsing. Requests are
limited to64KiB, reply batches to2MiB/128 envelopes and each envelope to1MiB.
Sixteen active requests are admitted; overflow is a local, not-submitted error.
Cancellation after admission preserves uncertainty. Close aborts and joins
owned requests; it does not claim rollback of a remote effect or native media.

The13 new HTTP-peer tests and full264-test TypeScript regression pass on local
Node26.7.0. These fixtures cover malformed replies, cross-resource correlation,
original identity, redirects, partial bodies, cancellation, capacity and cleanup.
Actual Connect production authorization/facade parity and Node22 qualification
remain open. Exact scope is in
`docs/evidence/gate-6/canonical-http-typescript.json`.
