# Server HTTP references

[The generated endpoint inventory](../../contracts/http-endpoints.json) maps all
current server/MCP router declarations to their references. It retains actual
source paths/methods and normalizes only placeholder names when matching an
OpenAPI path. This avoids treating `/v2/records/{id}/mutations` and its documented
`{uuid}` spelling as different endpoints. MCP and configured WSS/WebTransport
mounts reference protocol/configuration metadata separately.

The inventory checks28 declarations and29 explicit HTTP operations. Path
coverage alone does not qualify every `/v1/operations` body variant, response,
authorization case, profile or framework-generated method. Runtime evidence
remains attached to its particular contract and gate task.

| Interface | Reference |
| --- | --- |
| Canonical command/status/event batches | [canonical HTTP](../../contracts/openapi-canonical-http.json) |
| Current named application operation bodies and results | [application HTTP](../../contracts/openapi-application-http.json) |
| Trusted Identity/participant/Device/key/endpoint provisioning | [provisioning](../../contracts/openapi-provisioning.json) |
| Protected scoped credential issuance, Task grants and base artifact bytes | [protected HTTP](../../contracts/openapi-protected-http.json) |
| Backend-approved client enrollment, signed claim and revocation | [enrollment HTTP](../../contracts/openapi-enrollment.json) |
| Attachment control, links and receipt observations | [attachments](../../contracts/openapi-attachments.json) |
| Record redaction/deletion | [record mutations](../../contracts/openapi-record-mutations.json) |

Other named API families are linked per route in the inventory. Repeated
`/v1/operations` references describe different families; they are not a blanket
claim that retired ordinary communication variants remain supported.

Endpoint movement uses the existing canonical `connection.replace` command and
original-operation status reads. The [device replacement contract](../../contracts/device-replacement.md)
also specifies the physical target's native QUIC/WebTransport and WebRTC
allocation, negotiation and decoder-readiness continuations. These continuations use `connection.offer`
with distinct replacement profiles and the same original movement record.
Their bounded challenge and observation fields are published as `ReplacementReadiness*`
components in the canonical OpenAPI reference and the
[physical readiness schema](../../contracts/schemas/replacement-decoder-readiness-1.schema.json).
They add no HTTP media endpoint or HTTP readiness proof. The original source
remains active during preparation; a second peer connection becomes public only
after current authority, real resource readiness and movement commit succeed.

Enrolled credentials add an admission ceiling across finite application HTTP:
the actual Origin must equal the enrollment's exact browser Origin, including
the absence required by native enrollment. Explicit Task bearers retain their
enrolled backing credential's ceiling. This check grants no operation or Task
binding. Invalid bearer syntax/authentication remains with each route's own
decoding and authentication order; enrollment/Origin, capacity and storage
refusals can return the canonical `profile: "connect/2"` problem shape before
body decoding. The named application OpenAPI includes both rejection shapes.
See [remote enrollment](remote-enrollment.md) for physical QUIC/WT/WSS admission,
rotation and the pending public enrollment installers.

## Current application body schemas

`openapi-application-http.json` is generated from the real protocol DTOs with the
optional `connect-protocol/schema` feature. Deserialize and serialize contexts
remain separate, including optional/default/skipped fields and recursive Task
effects. Production builds do not enable that feature. The dispatch catalog
classifies59 top-level wire tags through69 selector cases, with all16 Task
commands represented separately. Current membership and receipt actions are
partitioned explicitly; ordinary canonical communication remains refused here.

`ApplicationOperation` describes currently named top-level calls. `WireOperation`
retains the full serde model for nested Task operations and deliberate refusal
checks. `RetiredTopLevelOperation` is a reference for HTTP501 refusals, without
fallback dispatch. Only the outer result-kind union is narrowed; nested Task
effects retain the complete attributed outcome model.

Schema validity establishes DTO shape. Current scope, resource audience, original
intent, business limits, lifecycle, provider configuration and Task/grant policy
still decide acceptance. Custom duplicate-key checks operate on JSON bytes and
cannot be expressed by a schema after parsing. Family-specific references retain
their policy details; this generated document supplies the consolidated current
body/dispatch model. Fresh service wire qualification is recorded separately.

```sh
python3 scripts/generate-application-http-openapi.py --check
python3 scripts/check-application-http-schema.py \
  --output .local/gate-6/application-schema-check
```

The generator builds the optional Rust schema example with locked offline
dependencies after verifying prepared sources. The checker needs Python
`jsonschema`; it compares synthetic schema vectors to the actual serde decoder.
These source/codec checks do not establish service authorization, media or remote
qualification.

The eight family references record current and retired selectors from this
consolidated dispatch table. Families with no current named selectors retain
their former success model as historical metadata, and advertise HTTP501 for
recognized retired calls. Original domain-policy notes and shared DTO components
remain available. Check their reconciliation with:

```sh
python3 scripts/generate-application-family-references.py --check
```

The isolated current server checkpoint validates 280 actual HTTP exchanges
against the references, including 69 root selectors, 16 Task command vectors,
authentication and framework refusals, plus selected real resource and replay
cases. [Recorded evidence](../evidence/gate-6/application-http-wire.json) states
the remaining provider, browser, lifecycle and host qualification limits. Run the
wire checker with a sealed CLI and matching frozen build evidence:

```sh
python3 scripts/check-gate6-protected-http.py --application-contract \
  --executable .local/gate-6/application-http/canonical-cli-01 \
  --build-summary .local/gate-6/application-http/build-01-summary.json \
  --build-inputs .local/gate-6/application-http/inputs-01.json \
  --output .local/gate-6/application-http/new-wire-check
```

## Scoped credential issuance

For client-owned keys and explicit Device/Endpoint/transport/Origin approval,
use the [remote enrollment workflow](remote-enrollment.md). Its protected
administration surface approves and revokes; the application surface accepts a
signed claim without a bearer. Original-ID recovery preserves the same scoped
credential and expiry. The reference distinguishes these implemented HTTP/Store
components from pending browser/native setup and transport-bound credential use.

`POST /v2/credentials` accepts a typed `CredentialIssueRequest` with the original
operation ID, target Identity, literal operation/recipient sets and absolute Unix
expiry. A current ordinary administrator must permit the target, recipients and
operations in the same tenant. Expiry cannot exceed the issuer's expiry or365days.
The request cannot issue administrator, wildcard or consent-delegation authority.
Caller-selected issuer, installation and human-assurance fields are refused.

Save the complete request before IO. Exact normalized replay under the original
current issuer recovers the same credential ID, bearer, issued time and expiry.
Changed intent or issuer conflicts; a revoked/expired child cannot be replaced by
replaying its old operation. The journal stores secret-free intent and a receipt;
the verifier and receipt commit atomically. Administrative issuance creates an
independent credential and does not claim human consent or a parent-revocation
delegation relationship.

The protected response uses `connect/2`. This route requires owned closing
HTTP/1.1 socket delivery, the concrete listener Host or `localhost:port`, and no
Origin, provider binding headers or Upgrade. Current administrator admission
precedes the64KiB JSON body limit/decoding. One five-second absolute deadline
covers admission, body and handler. Successful delivery retains authority and
shared capacity until physical socket IO ends, including after Body EOS; expiry,
disconnect and shutdown release it. Responses set `no-store`, `no-cache` and
`Connection: close`. An interrupted observation can leave a committed receipt:
recover with the original request rather than minting a new logical credential.

The [application SDK](application-sdk.md) exposes explicit issuance. Device and
signing-key enrollment remain separate trusted provisioning operations before
native admission. The [credential installer](credential-installation.md) saves
original intent and a fresh Device key before IO, enrolls the returned child and
confirms actual native admission before writing its completion marker.

## Protected Task access

`POST /v1/task-access` requires a current ordinary backing credential, exact
loopback Host and current `task_grant`/Task-owner authority. Origin and provider
headers refuse. Narrow model grants cannot mint or revoke this authority.
Native/HostSession binding registration and issuance commit together; verified
provider bindings belong to their own adapters.

Save the issuance request ID, complete definition and absolute expiry before IO.
Exact replay under current authority recovers the same grant, secret and expiry.
Changed intent conflicts; revocation cannot renew the old grant. Intended secret
responses use `connect-task-access/1` and disable caching. Business rejections
can use HTTP200 with `result.Err`; inspect both. Framework JSON/body/type failures
can be text responses. Neither response family is an ordinary Task tool result.

## Raw artifact bytes

The base artifact path has GET and PUT operations. GET returns actual
`application/octet-stream` bytes with `X-Connect-Offset`, `X-Connect-Eof`,
`Cache-Control: no-store` and `nosniff`. PUT receives opaque bytes and returns
`connect-control/1` metadata or a typed rejection. Query/body extraction and PUT
chunk-size checks precede ordinary bearer admission on these existing routes;
the protected Task exchange has its own admission order.

Keep a chunk's request ID, offset and exact bytes across uncertainty. Replay
retains the original receipt; it does not certify current file readiness. The
separate Finish API validates/publishes readiness. Reads and replays reauthorize
current owner/recipient/source scope; membership, an administrator label or a
Task grant cannot bypass the ordinary byte route's ceiling. Task delegates use
their explicit Task operation API instead.

The current configured chunk limit defaults to32768 bytes. The HTTP aggregate
body limit is the larger of that configured chunk limit and65536. Artifact names
allow at most255 UTF-8 bytes; response metadata and ArtifactBegin now reflect the
actual source bound, including a verified255-character ASCII name. Media types
are at most127 ASCII characters, and stored SHA-256 is lowercase64-hex.

## Checks and evidence

```sh
python3 scripts/generate-protected-http-openapi.py --check
python3 scripts/check-gate6-http-inventory.py --check
python3 scripts/check-gate6-protected-http.py \
  --executable "$CONNECT_VERIFIED_CLI" \
  --build-summary "$CONNECT_CLI_BUILD_SUMMARY" \
  --build-inputs "$CONNECT_CLI_BUILD_INPUTS"
```

The runtime check needs Python with `jsonschema` and a previously recorded current
CLI build. Its build/input evidence must match the supplied executable and current
source; it does not infer source identity from `--version` or build a distribution.
It creates fresh protected loopback state and strips provider/model credentials
from child environments. Grant-bearing wire samples stay in ignored `.local/`.

The earlier local [protected HTTP checkpoint](../evidence/gate-6/protected-http.json)
passes52 actual exchanges,51 checked against OpenAPI; the remaining Task-create
exchange is fixture setup. It also records five reference-sensitivity witnesses
and clean HTTP/MCP shutdown. This qualifies its local unsigned HTTP scope.
The [current facade completion](../evidence/gate-6/facade-completion.json) records 280
actual exchanges, all schema-checked without setup-only exclusions, plus current
CLI/MCP/scoped issuance and recovery checks. One supported actual-host rehearsal
has separate bounded evidence. G6-065 implementation is complete, including fresh
native consultation, eight handoff scenes and eight movement scenes. A reproduced
SQL-contention outage is fixed; historical audio-failure causation remains unproven
and is retained for G6-096/G6-098 qualification. Linux, remote/provider/media,
distribution and independent acceptance profiles remain separate.

## Operational health observations

The named application `health` operation includes `data.operational` only for a
currently authorized administrator with the separate `installation.observe` grant
and a recipient ceiling that includes `*`. A tenant administrator's `admin`
permission alone does not grant installation-wide observations.
Ordinary participant credentials may have `health` permission; that permission
alone does not expose installation-wide operation or resource counts. Current
revocation and scope checks apply on every read. Public `/healthz` remains a
minimal readiness result and does not sample these resource observers.

The operational object contains an observation timestamp, all configured limits,
operation-ledger state counts, optional runtime owner/task counts, and up to 32
distinct resource pools. It contains no operation IDs, intent/result bodies,
credentials, addresses, provider identifiers or private media. The six unsettled
operation counts are ledger states; they do not imply six classes of live workers.
Retained operation count includes terminal rows. Runtime `owned_tasks` includes
completed JoinSet entries awaiting a join; `physical_owners` counts retained
physical owners. These independently sampled values are not an atomic snapshot
or a process CPU/memory measurement.

Pool kinds distinguish native QUIC, WebTransport and WebSocket profile owners,
and actual HTTP/HTTPS connection, request and TLS handshake budgets for each
configured surface. Record-reference HTTPS has its own three budgets.
The shared primary/task MCP pool reports `mcp-connections` for physical TCP
sockets and `mcp-requests` for admitted request work. Its configured connection
limit applies before HTTP parsing; even incomplete headers occupy a socket slot.
Excess raw sockets close before authentication. Parsed request overload returns
HTTP 503 without entering authentication. Socket slots return only on actual
socket drop, independently of the request permit lifetime.
`used` reflects the actual owner or semaphore permit lifetime. Accepted/rejected
counters count admission attempts at that budget, including unsuccessful
authentication after a transport admission; they are not successful commands.
Separate budgets are not additive. Unconfigured or not-yet-instrumented pools are absent; registered
owners report `available` with usage, `busy` with null usage, or `retired` with
null usage. A null pool list means the registry was unavailable; a null runtime
means not configured or temporarily unavailable. Unknown values never imply zero.
The request serving a diagnostic read can itself occupy a request and connection
slot. Cumulative counters saturate at 9007199254740991; configured limits above
the supported exact numeric range are refused before Store creation/opening.
Resource-owner probes are weak references; diagnostics do not retain a listener
after its runtime owner is dropped.

This component is under G6-057 qualification. Its generated application OpenAPI
must accompany the exact Rust DTO changes. Capacity enforcement across all
resource classes, idle cleanup, process measurements and whole-task qualification
remain separately required by the implementation plan.

HTTP/HTTPS application and record-reference sockets expire after 30 seconds
without successful byte IO, or after a fixed 120-second connection lifetime.
Actual byte progress refreshes only the inactivity deadline; trickling incomplete
headers cannot extend the lifetime ceiling. Protected response authority deadlines
still apply earlier. During shutdown, protected bytes stop immediately; ordinary
routes have at most five seconds to flush a cancellation response before their
socket closes. Connection expiry never rolls back an accepted mutation: clients
must retain the original operation IDs and inspect/recover acceptance.

MCP sockets use the same 30-second byte inactivity and fixed 120-second lifetime
bounds. Listener cancellation allows at most one second for socket IO to drain,
inside the listener's five-second join deadline. Primary and task endpoints share
capacity, but shutting down one endpoint only drains its own sockets.

The G6-057 process fixture launches three explicit Linux process roots: the
production CLI server, an actual Rust typed SDK helper, and an owned Chromium
browser with its Node control driver. Its bounded read-only loopback profile
samples each process tree separately. Birth-time checks prevent PID reuse from
adopting unrelated processes. Public reports contain numeric CPU/RSS observations
and fixed role/state codes; they contain no command lines, environment, process
names, credentials or private payloads. Group RSS may count shared pages more
than once, and processes that exit between samples may be missed. These are
measurements for the selected fixture, not throughput or whole-gate guarantees.
Current runtime qualification is tracked in
[operational visibility evidence](../evidence/gate-6/operational-visibility.json).
