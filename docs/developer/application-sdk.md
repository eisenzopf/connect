# Explicit application SDK and default commands

Fresh setup can save both signed native and explicit local HTTP application
configuration:

```sh
connect init --directory .local/signed --example review-loop \
  --bind 127.0.0.1:4433 --http-bind 127.0.0.1:4434
connect serve --config .local/signed/server.toml
connect health --config .local/signed/operator.application.toml
connect resolve --config .local/signed/reviews.application.toml connect:agent@local
```

`operator.toml`, `reviews.toml` and `agent.toml` configure the signed
`canonical::CanonicalClient`. The corresponding `*.application.toml` files are
written only when initialization receives a concrete loopback HTTP port. They
configure `application::ApplicationClient` with that endpoint, exactly the issued
bearer, its original expiry and a request deadline. Both file formats are strict;
neither loader detects or falls back to the other format. Credentials and files
remain protected by the [fresh bootstrap](canonical-bootstrap.md).

Canonical communication uses typed `Payload` and `CommandScope` values through
the signed native `CanonicalClient` or explicitly configured
[`CanonicalHttpClient`](canonical-http.md). Neither selects a fallback transport.
Named application APIs use the existing
closed HTTP `Operation` contract through `ApplicationOperation::try_from`.
Canonical conversation create/list/send/history/close, snapshots/events,
Participant add/update/remove, operation recovery, transitions and Sessions are
refused by that constructor before transport effects. A failed or unsupported canonical command is never automatically retried through
HTTP or the named application adapter. The server also refuses canonical
Operation bodies on the generic application route and earlier native profile. Application reads, contact and
availability administration, attachment APIs, receipt preferences/delivery reports,
owned tasks and the other named application families remain independently
subject to current Service/Store authorization. Recognition is not a claim of
configured provider/media capability.

Trusted administration uses `ApplicationClient::provision` with the shared
`application::provisioning::ProvisioningCommand` and caller-owned operation ID.
The corresponding `connect provision` command reads that typed JSON from stdin
with an explicit administrator application config. See the
[trusted provisioning SDK](trusted-provisioning-sdk.md) for route mapping,
current-state replay and failure recovery. `connect revoke` now uses that same
service and application config with a required `--operation-id`; see
[credential revocation](credential-revocation.md).

Default `connect request` reads a bounded SDK `canonical::Command` from stdin:

```sh
connect request --config .local/signed/reviews.toml --envelope <<'JSON'
{"command":{"type":"conversation.list","payload":{"limit":10}},"scope":{}}
JSON
```

The SDK generates the envelope ID, timestamp and signature. Caller-selected
Conversation, Session, Connection, operation and revision assertions go in
`scope`; trusted principals and signatures cannot be supplied there. A successful
result has `kind: canonical_reply`, a fresh `data.request_id` and its verified
`data.frames` through the correlated ACK. An ACK completes the response batch;
operation results or receipts establish logical completion. Persist mutation and
message IDs with their exact intent before calling. Explicit retry reuses those
logical IDs and receives a fresh envelope correlation. Bounded native shutdown
gives QUIC time to drain after the response; a drain failure does not discard a
verified result or turn it into unknown acceptance.

For named application operations, select the interface and config explicitly:

```sh
printf '%s\n' '{"op":"health"}' | connect request --application \
  --config .local/signed/operator.application.toml --envelope
```

`--envelope` returns `profile` and a `result` containing `Ok` or a sanitized
`Err`; rejection also exits unsuccessfully. Without it, the command prints the
successful result. Inputs are bounded at 64 KiB. Prior native `Operation` JSON is
not accepted or translated by the default command. `health`/`diagnostics` now read
actual HTTP Service readiness, and `resolve` uses the named application API;
transport liveness is not substituted for health.

The application SDK disables environment proxies, redirects and idle connection
pooling. It accepts loopback socket addresses only, limits the request to 64 KiB
and the complete response to 2 MiB, checks the selected profile and expected result
kind, and does not echo raw server reasons or unknown error codes. One absolute
deadline includes response streaming, is limited by original credential expiry,
and cannot be renewed by ready chunks. Cancellation or timeout leaves acceptance
unknown. Retry the saved operation ID and intent; the SDK performs no logical retry.

Tasks require explicit discovery through permitted capabilities or a read of the
known bound task. `ApplicationClient::negotiate_tasks` and `negotiate_task` do not
mint authority. CLI task requests require `--application` plus `--task-address` or
`--task-id`. `ApplicationClientConfig::with_task_grant` replaces the bearer with
only the protected Native grant, rejects other binding kinds/revoked grants and
cannot extend the original local expiry. It never retains a backing credential
as a discovery or denial fallback. The backend independently checks the exact
current grant and task/operation scope on every request. The new SDK's unissued
replacement-token test proves refusal without owner fallback; it does not by
itself requalify every issued-grant or provider/host invocation profile.

The [scoped verification](../evidence/gate-6/application-sdk.json) covers actual
canonical runtime composition, new installed commands/configs, shared durable
receipts, cancellation/replay under SQL contention and current revocation, plus
controlled HTTP failures. The earlier `ConnectClient`/`ClientConfig`, media APIs,
setup/presence, credential/recovery and remaining example/task helpers still need
migration. Full canonical families/extensions, production media, refresh/step-up,
HTTP physical-write custody and broader hosting/transport/provider qualification
remain open. No full SDK migration or Gate 6 acceptance is inferred. Execution is
local macOS arm64 only.

Private incoming-contact administration uses
`ApplicationClient::incoming_contact_command` (or its cancellation variant) with
`application::incoming_contacts::IncomingContactCommand`. List/get return bounded
metadata. Resolve/reject retain the caller's complete stable logical decision
command across an uncertain response. Selection can name only an original
candidate under current account-owner, tenant, recipient, Endpoint, membership
and channel policy; administration grants no Conversation history. Voice
selection performs a fresh owned read of its original resource and queues the
actual incoming lifecycle, without dialing or accepting a Participant. See
[incoming-contact routing](incoming-contact-routing.md) and its
[HTTP reference](../../contracts/openapi-incoming-contacts.json).

Canonical clients can call `CanonicalHttpClient::transport_discovery()` or
`transport_discovery_with_cancel(token)` to obtain configured native control
limits and the optional WebTransport contact through authenticated
`GET /v2/discovery`. The response is bounded to8192 bytes during collection and
uses the configured request deadline. It supplies contact metadata; separately
configured TLS and enrolled signing trust are still required. See
[canonical transport discovery](canonical-http.md#canonical-transport-discovery).

## Retained Task diagnostics

`ApplicationClient::task_diagnostics(task_id, cursors, limit)` samples bounded
Task journal metadata, authorized source references, unassigned reply metadata
and process readiness. It uses the explicitly configured application credential;
it neither opens a native connection nor substitutes another authority. The
limit is1–100 and each lane has its own cursor. Missing optional permissions are
reported as unavailable sections. Invalid cursors, transport failures and a
revoked reader fail the sample. Reads do not consume replies or retry effects.

The first and final current Task reads must agree on Task/Conversation/owner
bindings. A revision change is reported; the pages are not an atomic snapshot.
Source content, message bodies, provider errors and credential plaintext are
removed from the returned diagnostic projection. Historical effect observations
remain distinct from current authorized EffectGet reconciliation.

The CLI uses the same public client:

```sh
connect task-diagnostics --config "$CONNECT_INSPECTION_APPLICATION_CONFIG" \
  --task-id "$CONNECT_TASK_ID" --limit 100
```

This command requires `ApplicationClientConfig`, including an explicit protected
loopback HTTP endpoint and the selected ordinary or narrow Native-grant bearer.
Signed native and minimal MCP config files are different formats and are refused.
An independently issued current owner inspection credential may read retained
work after an old agent/grant is revoked; it does not restore that delegation.
Revoking the inspection credential refuses its next sample, including final
rechecks after local host metadata is observed.

Current canonical-server behavior is recorded in
`docs/evidence/gate-6/canonical-task-diagnostics.json`. Historical Gate4/example
call sites have been moved to explicit application configuration; their own full
runtime profiles remain separate from this current canonical-server checkpoint.
