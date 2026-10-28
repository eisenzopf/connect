# Explicit Node HTTP control

Use `CanonicalHttpClient` for finite current `connect/2` commands and `HttpClient`
for named application operations. Both retain their default loopback HTTP mode.
Select `remote-https` explicitly to use a configured HTTPS origin:

```ts
import { CanonicalHttpClient, HttpClient } from '@rudeless/connect';

const control = new CanonicalHttpClient({
  endpoint: 'https://connect.example.test',
  credential,
  transport: 'remote-https',
});
const application = new HttpClient('https://connect.example.test', credential, 4000, 'remote-https');
try {
  const frames = await control.request('operation.get',
    {operation_id: originalId}, {cid: conversationId}, {timeoutMs: 4000});
} finally {
  await control.close({timeoutMs: 4000});
}
```

The endpoint and credential are captured once. Remote mode requires HTTPS with
platform certificate and hostname validation. URLs with user information, query,
fragment or a non-root path, malformed/oversized URLs, and invalid bearer values
are refused locally. Configuration errors omit the URL and credential. There are
no redirects, credential refreshes, automatic mutation retries or transport
fallbacks. Keep the original logical mutation ID across uncertain outcomes and
query that original operation before choosing any explicit replay.

The canonical route remains `/v2/commands`; application operations retain
`/v1/operations`. Existing framing, scope/correlation checks, response limits,
structured refusals, absolute request deadlines and owned canonical close remain
unchanged. Finite journal reads use `conversation.subscribe` with `live:false`.
Live native subscription leases, native Stream custody and incoming invitations
use the managed native SDK rather than finite HTTP.

The current local component suite passes109 functions, including four HTTPS
functions repeated against the exact packed and independently installed
136-file distribution. TLS itself is real; those route handlers are synthetic.
They prove trust/hostname refusal before HTTP admission, original intent capture,
actual refusal status, no redirect/retry, whole-body deadline and cancellation.

The production Linux candidate `node-https/1` runs both EdDSA and ES256. An installed
Node consumer talks to the original isolated HTTPS application listener with its
fixture CA installed before process startup. It compares one-effect replay,
finite journal facts and named receipt reads with an independent signed native
SDK record, then requires current revocation to deny read/replay/new mutation and
receipt data. SQL requires one original effect and zero forbidden effects; the
original Node process, native owner and server must join and drain. Four-second
requests, fifteen-second phases, ninety-second functions and five-second drains
remain required. Candidate execution is pending. No named remote endpoint or
provider/model account was contacted, and this does not close G6-062.
