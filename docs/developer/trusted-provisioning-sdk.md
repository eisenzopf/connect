# Trusted provisioning through the application SDK

Fresh installations with `init --http-bind` save a protected
`operator.application.toml`. `ApplicationClient::provision` uses that explicit
loopback HTTP endpoint and its bearer credential for the existing trusted
[provisioning API](../../contracts/openapi-provisioning.json). The server checks
current administrator, tenant and recipient authority before mutations and
receipt replay. Merely loading an operator-shaped config grants no authority.

Save a complete `ProvisioningCommand`, including its `operation_id`, before
sending it. After timeout, cancellation, lost response or `transport-uncertain`,
reuse that same command and ID. The SDK sends one transaction and performs no
automatic retries. Changing intent under the same ID produces a conflict.
Replays return the currently authorized resource projection: an endpoint or key
enrolled earlier may now be revoked. Replay never restores that resource.

```rust,no_run
use connect_client::application::{
    ApplicationClient, ApplicationClientConfig,
    provisioning::{IdentityKind, ProvisioningCommand},
};
# async fn example() -> connect_client::Result<()> {
let config = ApplicationClientConfig::load(
    std::path::Path::new(".local/signed/operator.application.toml"),
)?;
let mut client = ApplicationClient::open(config).await?;
let command = ProvisioningCommand::Identity {
    operation_id: "saved-enrollment-001".into(),
    alias: "reviewer".into(),
    kind: IdentityKind::Human,
    display_name: None,
};
let outcome = client.provision(command).await?;
# let _ = outcome;
# Ok(())
# }
```

The typed command selects one documented route. Credential revocation also has a
[dedicated CLI and recovery guide](credential-revocation.md):

| Command | HTTP POST route |
| --- | --- |
| `Identity` | `/v2/identities` |
| `Participant` | `/v2/conversations/{conversation_id}/participants` |
| `Endpoint` | `/v2/identities/{identity_id}/endpoints` |
| `VerifyEndpoint` | `/v2/endpoints/{endpoint_id}/verify` |
| `RevokeEndpoint` | `/v2/endpoints/{endpoint_id}/revoke` |
| `RevokeIdentity` | `/v2/identities/{identity_id}/revoke` |
| `SigningKey` | `/v2/identities/{identity_id}/signing-keys` |
| `RevokeSigningKey` | `/v2/signing-keys/{key_id}/revoke` |
| `RevokeDevice` | `/v2/devices/{device_id}/revoke` |
| `RevokeCredential` | `/v2/credentials/{credential_id}/revoke` |

Commands are validated and normalized before transport. Route targets `.` and
`..` are refused before HTTP can normalize them into another path. Endpoint capabilities
are sorted and deduplicated, and endpoint addresses use the shared canonical
form. Signing-key enrollment supplies public verification material and an
existing credential ID; it does not issue or recover a credential secret.
Returned resource kinds and immutable command targets must match. Revocation
requires a later revision and revoked state; current replay revisions may be
higher than the original result. A verification replay can show a subsequently
revoked endpoint.

`provision_with_cancel` uses the same 64 KiB request limit, 2 MiB streamed
response limit, credential expiry and absolute transaction deadline as named
application operations. Response decoding is inside that deadline. Redirects
are refused, environment proxies are bypassed, and diagnostics omit untrusted
server reasons and private input. Cancellation leaves acceptance uncertain;
the caller retains the saved command for recovery.

The CLI reads a single bounded typed command from standard input and always
prints a `ProvisioningResponse` envelope. Rejection also returns an unsuccessful
process exit status:

```sh
connect provision --config .local/signed/operator.application.toml <<'JSON'
{"type":"identity","operation_id":"saved-enrollment-001","alias":"reviewer","kind":"human","display_name":null}
JSON
```

The CLI rejects missing logical IDs, invalid route identifiers, oversized input
and native configs. It does not select another config or generate a replacement
ID. Normal signed Participant changes still use the canonical SDK; this trusted
administration interface is explicitly selected with administrator authority.

Local tests exercise the provisioning routes against the canonical runtime, original-ID
replay and intent conflict, cancellation during a SQLite write wait, current
administrator revocation, and installed CLI failures without private-input
echo. Existing server provisioning tests and OpenAPI exchange samples remain
separate authorization and contract evidence. This work does not qualify
credential secret issuance/recovery tooling, refresh/step-up, all high-level
consumers, HTTP physical-write custody, media, remote/provider profiles or Gate 6
acceptance.
