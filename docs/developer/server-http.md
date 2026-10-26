# Server HTTP references

[The generated endpoint inventory](../../contracts/http-endpoints.json) maps all
current server/MCP router declarations to their references. It retains actual
source paths/methods and normalizes only placeholder names when matching an
OpenAPI path. This avoids treating `/v2/records/{id}/mutations` and its documented
`{uuid}` spelling as different endpoints. MCP and configured WSS/WebTransport
mounts reference protocol/configuration metadata separately.

The inventory checks23 declarations and24 explicit HTTP operations. Path
coverage alone does not qualify every `/v1/operations` body variant, response,
authorization case, profile or framework-generated method. Runtime evidence
remains attached to its particular contract and gate task.

| Interface | Reference |
| --- | --- |
| Canonical command/status/event batches | [canonical HTTP](../../contracts/openapi-canonical-http.json) |
| Trusted Identity/participant/Device/key/endpoint provisioning | [provisioning](../../contracts/openapi-provisioning.json) |
| Protected Task grant issuance/revocation and base artifact bytes | [protected HTTP](../../contracts/openapi-protected-http.json) |
| Attachment control, links and receipt observations | [attachments](../../contracts/openapi-attachments.json) |
| Record redaction/deletion | [record mutations](../../contracts/openapi-record-mutations.json) |

Other named API families are linked per route in the inventory. Repeated
`/v1/operations` references describe different families; they are not a blanket
claim that retired ordinary communication variants remain supported.

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

The current local [protected HTTP checkpoint](../evidence/gate-6/protected-http.json)
passes52 actual exchanges,51 checked against OpenAPI; the remaining Task-create
exchange is fixture setup. It also records five reference-sensitivity witnesses
and clean HTTP/MCP shutdown. This qualifies its local unsigned HTTP scope.
Actual model-host, Linux, remote/provider/media and distribution profiles remain
separate. G6-065 remains open for credential issuance/recovery, full body-variant
parity and the remaining authorized facade behavior.
