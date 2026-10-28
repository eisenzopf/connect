# Trusted Node administrative SDK

`NodeAdministrativeClient` is a separate backend interface. `NodeClient` exposes
no provisioning method. Install the matching trusted `connect` executable and
supply an explicit SDK configuration with `[control]`, `tenant_id` and
`[application]`. The application service uses the exact same credential as
control. The current Rust administrative SDK supports the named local application
HTTP service; an explicit remote HTTP administrative facade remains unqualified.

```ts
import {NodeAdministrativeClient} from '@rudeless/connect';
const admin = new NodeAdministrativeClient({
  config: '/absolute/path/to/trusted-sdk.toml',
  executable: '/absolute/path/to/connect',
});
try {
  const command = admin.validateProvisioning({
    type: 'identity', operation_id: 'saved-administrative-id',
    alias: 'local-contact', kind: 'human', display_name: null,
  });
  // Durably save command.value and command.referenceJson() before I/O.
  const operation = await admin.provision(command, {
    operationId: command.operationId, timeoutMs: 4000,
  });
  const completed = await operation.wait({timeoutMs: 4000});
  const original = admin.operations.restore(operation.referenceJson());
  await original.status({timeoutMs: 4000});
} finally {
  await admin.close({timeoutMs: 4000});
}
```

`validateProvisioning` invokes `connect sdk-validate-provisioning` locally. It
loads no configuration, credentials, client or endpoint. The original Rust
`TrustedProvisioningCommand` parser normalizes and validates the complete shared
command enum. The resulting command tree and original reference are frozen and
branded. Save the explicit normalized command privately; it can contain private
labels and endpoint data. Default inspection and JSON serialization of the client/command redact the body; explicit command.value is the private export. Ordinary structural JSON cannot forge this token.

Submission uses `connect sdk-admin-stdio --config ...`, profile
`connect-sdk-administration/2`, and delegates to the existing public Rust
`AdministrativeClient`. The participant helper rejects administrative actions,
and the administrative helper rejects participant actions. Native and HTTP
configuration are fixed for this original worker lifetime. No parser, credential,
endpoint, transport or process fallback exists.

Local validation is bounded to two seconds, 64 KiB input and 128 KiB output.
Saved references have a 4 KiB bound and use the exact Rust operation-reference
parser without a connected client. Restore accepts only unscoped administrative
or separately identified cancel/reconcile references. Recover accepts an owned
reference without I/O; it restores no grant. Each current read or mutation goes
through the original SDK and host authorization. The administrative helper
allows 128 KiB wrapper framing while ordinary typed participant requests retain
their existing 64 KiB limit. The existing sixteen-exchange queue, absolute
request deadlines and two-MiB response bounds remain in force.

Administrative operations reuse the existing SDK operation class: captured
logical ID and full intent, stable host record, readiness/completion, typed
cancel/reconcile decisions and joined cleanup. A command/option ID mismatch or
nonempty mutation extensions is refused before submission. Started timeout,
cancellation or malformed helper output preserves the original mutation key and
uncertainty, fences the original worker and never retries automatically.

The administrative baseline passes 79 synthetic SDK functions, negative types,
installed-file equality and an installed public-consumer protocol with successful
recovery/replay/refusal/close and deliberate foreign-record failure. These are
protocol/process checks, not actual authority or cryptographic qualification.
The Ubuntu candidate uses all eleven real administrative command families,
independent Rust signed-journal comparisons, exact replay across a server restart,
changed-intent refusal, one original receipt and original resource drains. Its
runtime results remain pending. The added `node-administration/2` candidate includes ordinary-caller and current
revocation scenes; actual runtime verification and uncertain HTTP commit remain
required. See [the evidence](../evidence/gate-6/node-administration-sdk.json).

The current suite passes 81 component functions, including startup-context checks.
The managed helper profiles are version 2. Every response identifies
`startup` or `request`. An initialization failure proves no logical mutation was
submitted: the Node error retains its captured operation key and reports
`not-submitted`, while a read reports `read-only`. Request errors retain the Rust
SDK's actual certainty. Invalid phase, success labeled startup, or startup errors
claiming a recorded/uncertain result fence the original helper. The matching
current CLI and SDK are required.

The authority candidate reuses the original Rust fixture with an installed public
Node controller. An ordinary caller must be refused for submission and original
status. After revocation, the original owner must return no status data; a fresh
owner with that revoked credential must refuse both original-ID replay and a new
mutation before submission. A separately authorized client must recover the
same original journal record and value. SQLite must contain no forbidden/new
revoked receipt. No production authorization or native-close guard is relaxed.

The four-function profile requires both signing algorithms, twelve helper joins,
72 validator joins and six server joins. Four-second request, 15-second phase
and 90-second function deadlines remain. Seven installed synthetic protocols
pass, including unexpected deadline/success and foreign-record rejection; all
seven helpers and thirteen validators join. The runner retains only safe
observed error kind/certainty and source coordinates. Real Linux compilation and
behavior remain pending. See [the authority evidence](../evidence/gate-6/node-administration-authority.json).

The `node-administration/3` candidate adds the original held-response commit
fixture. The host must commit before Node cancels either wait. Queued work must
report `cancelled/not-submitted`; the sent original must report
`cancelled/uncertain` with its saved ID and reference. After joining the original
helper, a fresh client must recover the same signed record and value without
replaying the mutation. The proxy forbids a hidden second HTTP request; SQLite
requires one original effect/receipt and zero queued effects/receipts.

The combined six-function, two-algorithm candidate requires sixteen helpers,
eighty validators and eight servers joined. Nine installed synthetic protocols
pass with nine helpers/seventeen validators joined. A completed request cannot
be mislabeled uncertain. Real host commit, compilation and native qualification
remain pending. See [the uncertainty evidence](../evidence/gate-6/node-administration-uncertainty.json).
