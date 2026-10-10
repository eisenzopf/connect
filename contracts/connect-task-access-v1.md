# Connect protected task access, revision 1

`connect-task-access/1` is a developer credential exchange, separate from
`connect-task/1`, ordinary control responses, receipts and MCP tool outcomes.
Its local route is `POST /v1/task-access` on the application HTTP listener.
It is absent from the independently exposable Telnyx webhook listener.

The caller supplies a current backing bearer credential with `task_grant`
authority. Model invocation grants cannot authenticate this route. Issue takes
a caller-retained `request_id` and a `GrantDefinition`: task, Native or
HostSession binding, operation subset and absolute expiry of at most five minutes.
ProviderCall and ProviderChat require independently verified provider adapters;
this exchange cannot register or admit them as local origins.

The store checks current actor authority, task ownership/membership/lifecycle and
scope ceiling. Binding registration and issuance commit in one transaction.
A failed grant insert rolls back its binding. Matching issuance retries return
the same grant, secret and expiry under current authority. Changed intent under
the same ID fails with `request-conflict`, without registering another binding.
Revoked, cancelled or expired work cannot be revived by retry or refresh.
Refresh is an explicit new request under current backing authority.

Responses use `TaskAccessResponse { profile, result }`. Issued results contain
`ProtectedTaskGrant { grant, bearer }`; this is an intended secret exchange.
Its Debug representation redacts the bearer. Neither normal task results nor
grant inspection return it. Revoke takes only `grant_id` and returns that ID;
it does not cancel the task or close the conversation. Successful and error
responses disable caching.

The listener accepts loopback only. Host must match its concrete bound address
or `localhost` with the bound port. Present Origin, X-Call-ID and X-Chat-ID are
denied. Exactly one bounded Authorization value is required and removed before
handler dispatch. Admission reserves shared HTTP capacity before body decoding;
the access body is at most 64 KiB. Authentication and admitted body/handler work
have separate five-second deadlines and share owned listener cancellation.
Interrupted work returns `transport-uncertain` when a response can be delivered.
Dropping an HTTP future does not roll back a store transaction already running;
the caller reconciles the original issuance intent rather than inventing an ID.

With `[mcp] bind = "127.0.0.1:PORT"` and no binding field, the local MCP
listener admits Native/HostSession grants from their signed, registered rows.
Each request derives its task and origin from the protected grant; model fields
and provider headers cannot choose them. Supplying `[mcp.binding]` retains
fixed-origin mode. Neither mode turns a model grant into a backing credential.
Every request and execution rechecks current task/grant/actor authority.

`connect task-access issue` writes a private pending grant file before HTTP
dispatch; `resume` reconciles that same ID and inputs. An issued file contains
the bearer and is for protected host installation, never model context.
Existing output files are preserved. CLI stdout contains IDs, expiry and status
only; untrusted response prose is not echoed. An already-issued resume reports
`saved-not-refreshed` without claiming current server authorization. No automatic
renewal occurs. Requests use concrete numeric loopback addresses, no proxies,
no redirects, a five-second request timeout and bounded response decoding.

This interface enables local installation. Native SDK model-grant admission,
provider-origin installation and the actual installed-host continuation require
their own implementation and evidence in the Gate 4 plan.
