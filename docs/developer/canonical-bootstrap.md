# Default signed initialization and serving

Fresh `connect init` writes a signed canonical installation, and `connect serve`
runs its `uctp/2`, `connect/2` listener:

```sh
connect init --directory .local/signed --example review-loop --bind 127.0.0.1:4433
connect serve --config .local/signed/server.toml
```

Use `connect_client::canonical::CanonicalClientConfig::load` and the
[typed canonical Rust client](canonical-calls.md) with the generated
`operator.toml`, `reviews.toml` or `agent.toml`. The two application configs exist
when `--example review-loop` is selected. That preset currently creates enrolled
communication identities; the earlier review-loop application still awaits its
signed SDK migration. The connected-app preset is explicitly refused before
creation while its migration remains open.

With explicit `--http-bind 127.0.0.1:4434`, initialization also writes separate
protected `*.application.toml` files for the [application SDK and commands](application-sdk.md).
The saved HTTP port must be nonzero; no endpoint is guessed from a native port.

The installation contains a fresh Store, a TLS certificate/private key, a distinct
Ed25519 server signing key and raw public pin, and a distinct signing key for each
client. Trusted offline provisioning creates each Device endpoint and binds its
signing key to that exact credential, Identity and Device descriptor before the
config is published. Endpoint registration does not assert physical readiness.
Credentials, client signing keys and the server pin expire after 30 days. Restart
with explicitly current key material after expiry; a heartbeat cannot extend it.
Client paths resolve against their config, so the entire completed directory can
be moved without inferring a new trust pin from the network.

Initialization stages a new sibling directory, validates config/key enrollment
and Store integrity, protects directories as 0700 and regular files as 0600, then
publishes with an atomic no-replace rename. Existing files, directories and
symlinks are refused. Competing initializers cannot replace the winner. This is
fresh initialization; it does not convert or overwrite earlier installations.
A completed publish followed by a parent-sync failure reports that distinction.
Neither successful stdout nor errors print bearer tokens or private key contents.

The server uses the production authenticated profile and shared Service/Store
rather than stock Core dispatch. Optional `--http-bind` overrides the config's
loopback `http_bind` and serves the existing explicit HTTP application facade.
No provider or webhook flag is accepted by this command. Old server TOML is
refused; there is no parse fallback to a prior profile. The interim
`serve-canonical` command is removed. Use a concrete local port for saved client
configs; port zero is useful for a harness that reads the reported address and
sets its client endpoint explicitly.

Optional administrator-selected `[telephone]` configuration installs outbound
endpoint replacement on that same canonical Core before intake. It includes
explicit local SIP/RTP bounds, independent current Store account references and
configured confirmation policy. See [telephone endpoint replacement](telephone-replacement.md)
for the TOML fields, provisioning requirements and qualification boundaries.

The [bootstrap qualification](../evidence/gate-6/canonical-bootstrap.json) records
installed CLI execution on local macOS arm64, generated-key enforcement,
restart/replay, protected files, collision/refusal behavior and the production
runtime regression cases. It does not qualify crash/power-loss recovery of file
publication or every CLI workflow. The earlier `ConnectClient`, setup/presence,
credential/recovery tools, connected-app preset and example applications still
consume the earlier configuration and await migration. Default request now uses
canonical commands, while health/diagnostics/resolve and explicit application
requests use the new application SDK. Other helpers do not
establish signed-client support for this new default.
Remaining canonical/media and approved application families, auth refresh/step-up,
HTTP write custody, broader transport/hosting profiles and all Gate 6 acceptance
remain open. No deployment, live-provider operation or publication is performed.
