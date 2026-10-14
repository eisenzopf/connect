# Local first run

The provider-free flow needs the packaged `connect` executable and a writable private directory. It does not need a provider account, model account, task grant, tunnel, or public listener. It creates an operator credential and two ordinary revocable communication identities (`human` and `agent`). Task tracking is optional.

```sh
connect setup init --directory ./connect-local --bind 127.0.0.1:4433
connect setup inspect --directory ./connect-local
connect serve --config ./connect-local/server.toml
```

Run this separate rehearsal with a **new** directory. The executable starts and stops its own local server, exchanges messages and files in both directions through the public native QUIC client, pairs the authenticated identities, restarts at the same port, and verifies history and files. It prints a JSON result. It uses deterministic local participants; it is not an actual-model agent qualification or the video reference client.

```sh
connect setup demo --directory ./connect-demo --bind 127.0.0.1:0
```

Port zero selects a concrete available port during initialization. A repeat initialization validates complete state and preserves credentials. Occupied ports, an interrupted staging directory, an existing unversioned/partial installation, and permissive file modes fail with an actionable error. The versioned `setup.json` identifies paths and the schema. It contains no bearer secret. Existing Gate 4 state uses the existing explicit verified-backup migration commands; the first-run flow never adopts it automatically.

The `human.toml`, `agent.toml`, operator credential, database and TLS key require private modes (0600 inside a 0700 directory). Certificate paths are relative to the supplied client file. Never distribute these generated files as examples. Only the client credentials are exported to client/host installations; provider keys stay on the server.

## Native contacts

Pairing proves possession of current credentials for a named native identity. It does not prove ownership of a telephone number, add conversation membership, enroll a recipient for SMS, or grant additional communication authority. Both identities must already be permitted recipients. Challenges expire within ten minutes, allow at most five attempts, and are consumed once. The originating credential and pairing revocation are rechecked before replay.

`setup pair-start` takes the initiating client's `--config`, exact `--peer-identity-id`, stable `--request-id`, and a Unix `--expires-at`. Protect its JSON output. Reuse the same expiry and logical ID after uncertainty. `setup pair-complete` takes the independently authenticated peer's config, pairing ID and stable request ID; read the challenge from standard input so it is not an argument visible in process listings. `setup contact` reports native ownership separately from consent. `setup pair-revoke` revokes only the named pairing.

## Server secret references

A first run writes an empty protected `secret-references.toml`; no provider route is enabled. The macOS storage choice for an optional provider is a Keychain reference. The identifiers below contain no secret value:

```toml
version = 1
[references.VAPI_PRIVATE_KEY]
storage = "keychain"
service = "Connect/provider"
account = "vapi"
[references.TELNYX_API_KEY]
storage = "keychain"
service = "Connect/provider"
account = "telnyx"
```

Select the protected file with `CONNECT_SECRET_REFERENCES=/absolute/path/secret-references.toml` when starting server/provider workers or the setup catalog command. The named provider keys, Telnyx verification key, SMS policy token and configured SIP password names resolve through the same server boundary. With this file selected, missing references remain absent even if ambient variables exist. Locked, missing or denied Keychain entries fail without an interactive prompt or automatic plaintext fallback. `connect setup secret-set --references FILE --name NAME` reads a bounded value from standard input and rotates only the selected `Connect/` Keychain entry; `secret-remove` removes that exact entry. Neither command edits environment variables or prints values. Native qualification uses a child process and a disposable test keychain, never personal entries.

Development/CI may explicitly choose an environment reference:

```toml
version = 1
[references.VAPI_PRIVATE_KEY]
storage = "environment"
variable = "VAPI_PRIVATE_KEY"
```

Existing installations without `CONNECT_SECRET_REFERENCES` retain their supported environment-variable configuration. SIP provisioning checkpoints now contain only an installation/operation-bound vault reference and a protected one-way recovery check. Legacy plaintext is accepted only as migration input and is never serialized into a new checkpoint. Migration stores and reads back the exact original password before atomically replacing the checkpoint; no provider password is rotated. Missing/locked vaults preserve the original intent and fail without fallback. Native Keychain verification is separate from mock/CI evidence.

## Migrate or restore a SIP checkpoint

Use current operator authority from the same installation. Existing `providers apply` and `providers voice-apply` flags continue to work with protected `operator.toml` and `server.toml` next to the selected checkpoint. For a different explicit layout, add `--config` and `--server-config`. Both commands migrate legacy SIP secrets before provider work. The generated secret is stored only in the server Keychain; the checkpoint never exports it.

```sh
connect providers vault-migrate --state ./connect-local/provider-setup.json --config ./connect-local/operator.toml --server-config ./connect-local/server.toml
```

This command performs no provider request. It is idempotent and retains resource IDs, uncertainty state, names and logical operation IDs. A crash after vault acceptance but before checkpoint replacement reuses the same entry on retry. An existing entry with another password is rejected rather than overwritten. An uncertain/existing SIP resource with no original credential is rejected rather than assigned a newly generated password. An interrupted legacy `.pending.json` needs explicit checkpoint recovery; do not delete receipts or silently repeat creation.

Backups/restores retain the installation identity, so vault references continue to name the original namespace. If the Keychain entry is unavailable after moving/restoring the installation, import the **original** password through standard input:

```sh
connect providers vault-restore --state ./connect-local/provider-setup.json --config ./connect-local/operator.toml --server-config ./connect-local/server.toml
```

The protected recovery fingerprint rejects a different password before any vault insertion. Restore cannot change the logical reference or rotate a provider password. Keychain data is not exported in a database backup; retain its approved secure recovery mechanism. The optional `--keychain /absolute/path` selects an explicitly supplied private file-based Keychain (0600); qualification uses disposable test paths only. Expired, revoked, non-admin or mismatched installation authority denies migration and secret reads, including reuse. No personal Keychain entries or state are changed by qualification.

## Optional providers and honest readiness

`connect setup provider-discover --provider vapi --output CATALOG.json` reads only the Vapi catalog and resolves only its selected server reference. Use `--provider telnyx` for Telnyx or `--provider all` (the default) for both. The legacy `connect providers discover` command accepts the same selector. Single-provider discovery produces a partial combined catalog (`complete: false`), permitting exact observed resource reuse while refusing creates until complete discovery; omitted providers supply no resources or capabilities. This is an explicit account read, never part of the account-free demo. It fetches bounded Telnyx pages, strips raw provider DTO/config/secret fields, and records catalog completeness. Qualification uses injected loopback clients only.

`connect setup provider-preview --catalog CATALOG.json --intent INTENT.json` performs no requests or mutations. The public Rust `connect_connectors::provisioning` module defines `Catalog`, `Intent`, exact ID `Reuse` selections, explicit resource `Create` selections, and prerequisite gaps. Reuse never edits the selected asset. Number purchase is rejected. Ambiguous IDs, rejected authentication, inaccessible selections, missing capabilities and truncated catalogs are explicit. The retained `providers plan` preview now names exact existing assistant, messaging profile, trunk and number IDs before optional association changes.

The public `Provisioner` adapter boundary checkpoints immutable intent and per-step logical mutation IDs before an attempt. After a crash or unknown response, execution reconciles before sending again. Definite failure, uncertainty, acceptance and cancellation remain distinct. Cleanup returns only accepted owned creations and refuses unresolved outcomes; it never deletes unrelated account assets. The new boundary is qualified with local fixtures; it is not a live account qualification.

`connect setup readiness --observations OBSERVATIONS.json` produces separate inbound/outbound actions for authentication, resources, callbacks, SIP registration, RTP, media and SMS eligibility. Absent observations are unknown, expired observations stale. HTTPS success cannot establish SIP/RTP readiness, native contact ownership cannot establish SMS consent, and readiness never proves human receipt or silently chooses another route.

Third-party enrollment services implement the protected server `SmsPolicy` boundary or use `HttpSmsPolicy` and the authenticated complete `SmsGrantSnapshot` schema. `providers::Worker::with_policy` accepts that adapter. All services synchronize through the same store lease/evidence boundary: STOP cancels queued work and blocks dispatch/replay; START permits only new authorized work; stale or unknown eligibility fails safely. Signed provider control callbacks retain the existing HELP/STOP/START handling. Agents cannot supply enrollment evidence through ordinary operations.

## Optional Codex/model and app journeys

After first-run initialization, stop its server before exporting the optional Codex path:

```sh
connect setup agent-install --directory ./connect-local --mcp-bind 127.0.0.1:8877 --output ./connect-codex
connect serve --config ./connect-local/server.toml
connect agent-access run-codex --installation ./connect-codex --prompt-file ./prompt.txt
```

This exports the existing normal revocable agent key, MCP override and skill to a new private directory. Existing host configuration is preserved. The qualified local host is `codex-cli 0.162.0-alpha.17.2`. Before saving a turn intent, the adapter probes `--version` and `login status` with bounded process ownership and no model invocation. Select and authenticate the Codex/model host explicitly using the existing agent-access instructions; an unknown version or missing model authentication fails rather than substituting another host or identity. Subsequent runs resume the exact saved Codex session explicitly. Connect inbox events do not imply automatic model wake.

`connect agent-access remove-codex --installation ./connect-codex` removes only the verified Connect credential/configuration/skill files. It preserves unrelated files, saved session state and diagnostics. Revoke the corresponding server credential separately when removing its authority.

For the separate app example, retain the packaged `connect init --example connected-app` and review-loop guide. The developer-owned app uses public interfaces and keeps its own contact/business policy. The no-account demo above is a reproducible product check; independent fresh-user timings, actual model qualification and installed-candidate proof remain separate release checks.

A verified migration also updates the existing first-run `setup.json` schema after the database upgrade. The original metadata must match the verified backup and installation paths; credentials and secret references remain unchanged. If the database commits but metadata publication is interrupted, preserve the state and make a **new verified backup of that exact current state** with the current administrator, then run the explicit migration command against that backup to reconcile metadata. The old pre-upgrade backup cannot qualify this repair, and ordinary setup inspection never adopts or rewrites mismatched state.
