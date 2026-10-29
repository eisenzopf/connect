# Trusted identity and endpoint provisioning

The trusted backend creates identities, conversation appearances and verified
endpoint bindings through the application service. Endpoints belong to an
identity/device and can serve multiple conversations. They contain a server-owned
secret reference and an attributed route owner. Public communication credentials
cannot provision or verify endpoints.

The implemented HTTP subset is described in
[OpenAPI](../../contracts/openapi-provisioning.json). The full canonical command,
SDK and remote hosting references remain assigned to later Gate 6 tasks.

Use the configured loopback HTTP listener and a server-side operator credential
whose scope includes `admin` and the intended recipients. Creating a new identity
requires wildcard recipient authority because its ID is generated at admission.
The operator remains outside that identity's conversations unless explicitly
admitted. Provisioning does not grant the operator access to message history.

```sh
curl --fail-with-body "$CONNECT_HTTP_URL/v2/identities" \
  -H "Authorization: Bearer $CONNECT_OPERATOR_TOKEN" \
  -H 'Content-Type: application/json' \
  --data '{"type":"identity","operation_id":"create-contact-1","alias":"organizer","kind":"human"}'
```

The JSON response contains `profile: "connect/2"` and `result.Ok` or `result.Err`.
A successful identity result includes `identity_id`. The backend uses that ID in
its next request; it does not put the operator credential into a browser client.
For example, with `IDENTITY_ID` set to the returned ID:

```sh
curl --fail-with-body "$CONNECT_HTTP_URL/v2/identities/$IDENTITY_ID/endpoints" \
  -H "Authorization: Bearer $CONNECT_OPERATOR_TOKEN" \
  -H 'Content-Type: application/json' \
  --data "{\"type\":\"endpoint\",\"operation_id\":\"provision-phone-1\",\"identity_id\":\"$IDENTITY_ID\",\"device\":{\"id\":\"device-organizer-phone\",\"kind\":\"telephone\",\"platform\":\"pstn\"},\"transport\":\"telephone\",\"address\":\"+12025550101\",\"capabilities\":[\"audio\",\"dtmf\"],\"secret_ref\":\"env_TEST_TELNYX_KEY\"}"
```

This example uses a reserved fictional destination and creates local configuration
only. It does not place a call. The reference must identify operator-controlled
secret configuration; no key or password belongs in `address` or `secret_ref`.
Secret resolution, route selection, consent and actual provider/media readiness
are separate prerequisites. Telephone/SMS, SIP and Vapi endpoints require a secret
reference. Native QUIC and browser bindings may omit it.

Provisioning returns an **unverified** endpoint at revision 0. A trusted verifier
records its evidence separately:

```json
{
  "type": "verify_endpoint",
  "operation_id": "verify-phone-1",
  "endpoint_id": "RETURNED_ENDPOINT_ID",
  "expected_revision": 0,
  "evidence_ref": "trusted-ownership-proof-1"
}
```

POST that body to `/v2/endpoints/{endpoint_id}/verify`. Evidence is a server-side
proof reference, not a client-supplied `verified` boolean. Endpoint ownership
verification does not establish SMS enrollment, call consent, reachability or
bidirectional audio readiness. Unsupported or unknown provider capabilities must
remain explicit in the later routing/dispatch paths.

A backend can POST a `participant` command to
`/v2/conversations/{conversation_id}/participants` with `operation_id`,
`conversation_id`, `identity_id` and a `role`. Each conversation receives a
different participant ID. Roles do not grant content or administrative authority.
Existing private history is not shared by this operation; audience projection
and explicit history sharing are enforced by the membership/message work.

To revoke, POST `revoke_endpoint` to `/v2/endpoints/{endpoint_id}/revoke` or
`revoke_identity` to `/v2/identities/{identity_id}/revoke`, including the target ID,
a new `operation_id` and its `expected_revision`. Endpoint revocation removes its
secret reference and cannot be undone by a late verification. Identity revocation
invalidates credentials, devices and endpoints together. New credential issuance
for a revoked identity is refused. Active-media teardown remains an obligation
of the later current-authority service and transport tasks.

Reuse the same logical operation ID after losing a response. The same canonical
intent returns the same resource, projected at its current state. Changed intent
under that ID returns 409. Duplicate live address claims or foreign device claims
also conflict. Addresses and secret references are omitted from responses.
Expired receipt results retain a digest tombstone and return 410, never execute
again. Credential and recipient scope are checked again on replay.

Administrative routes reject browser Origin and provider call/chat identity
headers, authenticate before body decoding, bound bodies to 64 KiB and bound the
whole admitted request to five seconds. Capacity exhaustion returns 503. A timeout
or shutdown may leave a committed mutation: recover with its original operation
ID. HTTP success here proves local provisioning, not a working provider route.

Current state uses schema 115, which adds durable client enrollment. Schema 80
introduced the explicit tenant record-reader roles described below.
Earlier candidate databases require a verified backup with their matching binary
and explicit initialization of a new state path. Opening incompatible state never automatically converts or resets it.
See [backup and reset instructions](../decisions/014-gate-6-contract-version.md).

Run `python3 scripts/check-gate6-provisioning.py` for isolated store/HTTP evidence.
The fixtures perform no external provider calls.

Tenant vCon-read role assignment, revocation, private-source policy and revision
checks are described in [tenant-record-readers.md](tenant-record-readers.md).
The role uses the same protected provisioning SDK/CLI and durable receipts.

Contributed message Endpoints use `transport:"connector"`, a protected
`secret_ref`, and messaging capabilities only. Their locator is
`connector://<registered-name>/<encoded-backend-target>`. Normalization decodes
exactly once and canonicalizes equivalent escapes before the immutable operation
intent is recorded. Userinfo, port, query, fragment, invalid UTF-8 and decoded
whitespace/control characters refuse. This is a declared locator, not a network
connection or principal grant. Trusted startup must separately install the exact
manifest, driver, account custody and verified Endpoint route. SMS keeps its
explicit profile/source/enrollment policy; native media uses its own transport.
