# Scoped credential installation and recovery

`connect issue` creates a separately scoped credential, a fresh Device/key binding
and protected native/application files. Select a current administrator application
configuration and a trusted canonical native contact template. The template
supplies the native endpoint, server name, TLS trust and server signing pin. Its
credential, Identity, Device and private signer are not installed or used; its
old private key need not exist.

Create the target Identity through trusted provisioning first. Save a private
`CredentialIssueRequest` JSON file with `operation_id`, `identity_id`, literal
`operations`/`recipients` arrays and absolute Unix `expires_at`. For example,
replace the target/peer placeholders with actual Identity IDs:

```sh
umask 077
python3 - <<'PY' > .local/agent-request.json
import json, time
print(json.dumps({
    "operation_id": "saved-agent-issuance-1",
    "identity_id": "TARGET_ID",
    "operations": ["send", "history", "operation.get"],
    "recipients": ["PEER_ID"],
    "expires_at": int(time.time()) + 86400
}))
PY
connect issue --config .local/connect/operator.application.toml \
  --native-template .local/connect/operator.toml \
  --request .local/agent-request.json --signing-algorithm EdDSA \
  --output .local/issued-agent
```

The output must be a new directory with an existing parent. The administrator
must currently permit the target, operations and recipients in the same tenant.
Expiry must be future, within365days and within the actual issuer lifetime.
Administrator, wildcard and consent-delegation scopes refuse. This is independent
administrative issuance, without human-consent or parent-revocation delegation
claims. `--signing-algorithm` chooses `EdDSA` (default) or `ES256` for the fresh
Device key and preserves the selected server signing pin.

The installer saves normalized intent and its fresh key before network IO.
Intent binds the original issuer's token hash, concrete application endpoint,
native contact, key validity, public key and asset hashes. Enrollment operation
IDs derive from the issuance ID and remain stable. The Device advertises
control/data capability; media still needs configured physical endpoints,
capabilities and current policy. DER or configured PEM trust roots retain the
native SDK's bounds:256KiB total, at most8 certificates of at most64KiB each.
Other installation inputs/assets are bounded at64KiB.

| File | Purpose |
| --- | --- |
| `intent.json` | Original request, contact, issuer and asset bindings |
| `prepared.json` | Exact intent hash published before the first network mutation |
| `device-signing.der` | Fresh private Device signer |
| `certificate.der`, `server-signing.pub` | Exact configured public trust/pin bytes; the trust file may contain PEM |
| `issued.json` | Protected original child credential and server-derived bindings |
| `client.toml` | Canonical native configuration using the child credential |
| `application.toml` | Application configuration using the same child/expiry |
| `install.json` | Completion marker and hashes after actual native admission |
| `issue.lock` | Exclusive local installer lock |

Private ordinary files are published with synchronized temporary files and
atomic no-replace rename. The qualified filesystem implementation is macOS/Linux.
Interrupted temporary files are not adopted as intent. The preparation marker
refuses edited saved intent, including a changed logical ID after lost delivery.
Completion is written
only after actual native authentication and joined shutdown, followed by asset
rechecks. It proves that admission occurred, not continuing authority or media.
Only non-secret IDs and status are printed; keep the directory private.

## Recover the original installation

After an uncertain or failed call, retain its directory and use:

```sh
connect issue-resume --config .local/connect/operator.application.toml \
  --installation .local/issued-agent
```

Resume requires the same current issuer credential and application endpoint.
It verifies saved key/trust/configuration hashes, replays original issuance and
enrollment, and confirms native admission. Committed lost responses recover the
same credential, Device, key and absolute expiry. If the native listener was
temporarily unavailable, restore that same configured endpoint before resuming.
Completed installations can also be rechecked this way without changing files.

Changed issuer/endpoint, altered assets, inconsistent partial outputs, revoked
credential/key/Device, expiry or failed native admission refuse. Existing assets
and unrelated notes remain intact. An initial preparation without both intent
and its preparation marker cannot resume; no network mutation occurs before
both exist. Diagnose its
preserved state before explicitly starting a separate issuance. The installer
does not automatically revoke, renew, replace, clean up or select fallback
authority. New client files contain only the child bearer and fresh signer.

Use `client.toml` for native request/stdio and `application.toml` for permitted
named APIs. The child has only requested permissions. Its native file can also
supply `agent-access export-codex --config .local/issued-agent/client.toml` with
the selected server config, MCP address and a new export output directory. That
export preserves the child credential and copies minimal MCP/skill assets without
Device keys or native trust; see [agent access](../AGENT_ACCESS.md).

The former generic `issue --server-config/--identity-id/--voice/--tasks` interface
and generic single-file output are replaced without a format fallback. See the
[protected HTTP contract](server-http.md) for authority and delivery semantics.
