# Trusted app provisioning and embedding

This example creates a System identity for the app, a Human identity and an Ai
identity, binds one owned native/browser device and public signing key for each,
and issues explicit client permissions restricted to those three identities.
An Ai identity does not start a model or provider call.

Run setup in a trusted backend beside the server. The example uses only the
public `connect_client::application::ApplicationClient` provisioning and scoped
credential APIs. It never opens Connect storage or calls a media bridge.
The administrative config and any provider secret references remain server side.
See the [provisioning contract](../../docs/developer/provisioning.md) and
[HTTP/OpenAPI reference](../../contracts/openapi-provisioning.json).

Before building, run `bash scripts/prepare-dependencies.sh`. Build with:

```sh
cargo build --offline --locked -p connect-server --example provision_app
```

Prepare a protected ApplicationClient TOML config with the existing loopback
administration address, administrative credential, absolute expiry and bounded
timeout. Keep it under ignored `.local/`; do not use a participant credential.
Use a fresh private state directory:

```sh
target/debug/examples/provision_app setup \
  .local/app/admin.toml .local/app/plan.json .local/app/setup
```

Copy [plan.example.json](plan.example.json) into your private .local directory.
Its zero timestamps and placeholder public keys deliberately refuse before any
mutation. Replace them with your chosen absolute window, owned client public
keys, verified enrollment evidence and actual configured transport addresses.
No Connect Identity, Endpoint, Conversation or Session IDs need to be copied.

The JSON plan has `setup_id`, absolute `not_before`/`expires_at`, and `app`,
`human`, `assistant` objects. Each object contains an alias, display_name,
device {id,kind,platform}, transport (quic/webtransport/webrtc), address,
capabilities, key_id, algorithm (EdDSA/ES256), public_key and
enrollment_evidence. Public keys are unpadded base64url verification material.
Private signing keys stay with their client owners. Use the actual enrollment
ownership evidence established by your backend; a browser-posted identity,
endpoint or evidence string is not proof of ownership. The approved enrollment
workflow is described in [browser enrollment](../../docs/developer/browser-enrollment.md).

The fixed grant catalog lists current messaging, Session, movement and Stream
controls in setup.rs. Recipients are the other two exact identities; no wildcard,
administrator, consent delegation or provider secret is issued. Current
Conversation membership, role, Endpoint and operation authorization still apply
to each action. Change that backend policy deliberately for a different app.

The backend fsyncs the complete original request before each mutation. If a
response is lost, stop and explicitly rerun the same command and saved plan.
Rerun reuses the original IDs, revision assertions and absolute expiry, including
when an Endpoint is now verified. It queries the installation identity and
refuses a different installation or changed plan. A private file lock prevents
two setup processes from racing. Every replay goes through current server
authorization; revoked/expired grants remain refused.

Private output `enrollments.private.json` contains scoped client credentials.
Deliver only the selected actor's enrollment through your authenticated client
bootstrap; never expose this entire file or the journal through a static server.
The separate `contacts.json` contains public contact labels and Identity IDs.
Copy only that public file and contacts.mjs to the app's static asset root.

With an already enrolled BrowserSdkClient, let users choose a contact:

```js
import {mountContacts} from './contacts.mjs';
const contacts = await (await fetch('./contacts.json')).json();
mountContacts(document.querySelector('#contacts'), contacts, async contact => {
  const operation = await client.conversations.create({
    policy: 'persistent', participants: [contact.identityId],
  }, savedMutation);
  showConversation((await operation.wait({timeoutMs: 5000})).value);
});
```

Persist each logical mutation and its intent before the callback submits it.
The contact choice supplies the returned public Identity ID programmatically;
users do not edit routes or copy Conversation/Session IDs.

For public in-process composition, the same executable supports:

```sh
target/debug/examples/provision_app serve .local/app/canonical-server.toml
```

The source targets the approved macOS and Linux hosts. It uses
CanonicalServerConfig::load, RunningCanonicalServer::start and the owned
shutdown API. Normal config controls listeners and protected server-only secret
references. This command runs configured services; the verification fixture uses
only isolated local configuration with provider adapters disabled.

## Verification

The local source fixture passes EdDSA and ES256, including a committed lost
Identity response, all15 original setup requests on explicit rerun, current
Endpoint revocation, client/browser provisioning refusal, private file modes and
contact-based signed Conversation creation. Both hosts and proxy workers join.
The runnable example also builds. See [bounded evidence](../../docs/evidence/gate-6/provision-app-example.json)
for failed intermediate checks and scope limits.

    cargo test --offline --locked -p connect-server --example provision_app -- --test-threads=1
    node --check examples/provision-app/contacts.mjs

Combine one actor’s protected enrollment with that client’s own signing key and
your trusted server/tenant bootstrap configuration. Contacts are public lookup
data and carry no authority.
