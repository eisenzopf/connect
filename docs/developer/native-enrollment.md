# Native client enrollment

The public Rust `connect_client::enrollment` API enrolls a fresh QUIC client.
The client receives a bounded approved offer from a trusted backend; it never
receives the backend administrator credential or provider keys. Browser
installation is described in [browser enrollment](browser-enrollment.md).

Generate an Ed25519 or ES256 `NativeEnrollmentKey`, save it to a private file,
and save the complete original `EnrollmentRequest` before asking the backend
for approval. Put `key.public_key()` and `key.algorithm()` in that request.
Use explicit approval and completion operation IDs, Identity/Device/key IDs,
Endpoint address/capabilities, literal scopes/recipients and absolute expiries.
The native profile is `EnrollmentTransport::Quic` with no browser Origin.
The backend approves through the administration endpoint described in
[remote enrollment](remote-enrollment.md); only the public key leaves the client.

`NativeEnrollment::create(directory, offer, expected_request, key)` normalizes
and compares the entire approved request with the expected original intent,
checks the client key, then saves the offer and PKCS8 key in a new private state
directory. On Unix the directory is mode0700 and files are mode0600. Existing
state is never replaced. Files are synced before atomic exclusive publication,
so a partial write cannot become the saved offer or installed config. Use ignored
`.local/` for repository rehearsals.

`EnrollmentClient::open(EnrollmentClientConfig { origin, trust_certificate,
timeout_ms })` accepts an exact HTTPS origin, with platform certificate trust
and an optional explicit PEM root. Explicit loopback HTTP origins are supported
for isolated local use. Credentials, paths, queries, fragments, default-port
aliases and redirects are refused. The enrollment client sends neither bearer
nor Origin headers. It uses closing HTTP/1.1, bounded JSON and one deadline that
includes response collection and decoding. No automatic retry occurs.

Call `client.claim(&saved).await` or `claim_with_cancel`. After uncertainty or a
process/server restart, load `NativeEnrollment::load(directory)` and explicitly
claim again. The saved offer, private key, challenge and completion operation ID
remain unchanged; ES256 signatures may differ while proving the same context.
An expired challenge remains loadable because the server may recover a completed
original claim under current authority. Fresh expired claims still refuse at
the server. Cancellation or timeout does not prove that the claim failed.

A successful response must match every approved binding, original completion ID,
issuer Identity, installation, exact scope/recipient sets and absolute expiry.
The SDK refuses substituted or expired credentials. Server error messages and
details are discarded; local errors contain a closed error code and fixed text.
Debug output for the key and saved enrollment is redacted. Deliberate offer/claim
serialization contains protected challenge/proof material and must stay private.

Supply independently trusted `EnrollmentServer` contact, TLS root and server
signing key to `saved.install(&credential, server)`. The method writes private
`client.toml` beside the saved original state. It retains the already-saved
client key and fills Identity/Device/key/bearer fields from the verified offer
and response. It does not copy an administrator credential. Reinstalling the
identical config is idempotent; a different config cannot overwrite it. Load the
result with `CanonicalClientConfig::load` and open a signed `CanonicalClient`.
The server checks current enrollment transport, key, Device and Endpoint again.

Rotation creates fresh original IDs and a fresh key/state directory. A backend
may approve the exact existing Device/Endpoint for the replacement and revoke
the old enrollment after installation. Keep the old state for original receipt
reconciliation; never replace its key to bypass current authority.

This API does not prove Endpoint reachability, provider consent or media
readiness. Named remote/Linux hosting and release qualification remain separate Gate 6 tasks.

[Native component evidence](../evidence/gate-6/enrollment-native-client.json) records
the actual public API and local failure/recovery checks, including both algorithms.
