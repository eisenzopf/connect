# Credential revocation and durable recovery

`connect revoke` uses the current explicit HTTP application configuration and
trusted provisioning service. Start the signed runtime with its local HTTP
facade, save a logical operation ID, and revoke an immutable credential ID:

```sh
connect revoke --config .local/signed/operator.application.toml \
  --operation-id saved-credential-revocation-001 CREDENTIAL_ID
```

The command always prints the typed `ProvisioningResponse` envelope and returns
an unsuccessful exit status for rejection. The old `--server-config` database
argument is removed. Native configs are refused. An operation ID is required;
the CLI never generates a replacement after uncertainty.

The same mutation is available through `ApplicationClient::provision` and
`connect provision` with this typed JSON:

```json
{"type":"revoke_credential","operation_id":"saved-credential-revocation-001","credential_id":"CREDENTIAL_ID"}
```

It posts to [`/v2/credentials/{id}/revoke`](../../contracts/openapi-provisioning.json).
The URL and body targets must match. The closed command shape rejects supplied
Identity claims or revisions. Credential IDs are immutable, and revocation is
terminal, so no expected revision is needed. The response includes only the
credential ID, owning Identity ID, original expiry and revoked state. It never
returns a bearer, verifier, scopes or refreshed authority.

Server admission derives the target owner from the current credential row and
acquires its Identity write fence. The transaction rechecks current administrator,
recipient and tenant authority before changing state or replaying a receipt.
Owned composition reuses a sufficient write guard; an insufficient or foreign
proof is rejected without falling back to a second admission. A busy writer is
an effect-free refusal and can be retried with the same operation ID.

Save the entire command before submission. After cancellation, timeout or lost
response, retry that exact ID and intent with a current authorized administrator.
The shared ledger returns one receipt. A changed target under the same ID
conflicts. Expired receipt tombstones prevent reexecution. Canonical signed
`operation.get` exposes the same receipt under current administrator, recipient
and tenant checks; recovery never restores the revoked credential.

Self-revocation can commit and return completion. That credential cannot then
replay its own receipt. Another current administrator credential for the same
owner Identity can recover the original logical operation. A different Identity
cannot adopt its operation ID. Keep any recovery administrator credential under
existing protected local administration; this endpoint does not issue one.

Revocation invalidates fresh application calls and signed admission. The local
canonical runtime also retires an already-connected signed peer backed by the
revoked credential. Revoking one credential does not grant new authority or
revoke another credential by reusing its signing key.

Selected local macOS arm64 checks cover logical replay/conflict/tombstones,
current tenant/recipient/admin refusal, owned/busy/foreign proofs, expiry after a
SQL wait, self-revocation and current replacement recovery, HTTP target/body
binding, actual OpenAPI exchanges, installed CLI failures, signed peer retirement
and canonical recovery. This does not close Gate 6 acceptance. Credential secret
issuance/recovery, refresh/step-up, remaining high-level consumer migration,
production media and physical HTTP response-write custody remain open. Broader
workspace and remote/provider profiles are not claimed by this scoped check.
