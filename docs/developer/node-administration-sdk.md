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
`connect-sdk-administration/1`, and delegates to the existing public Rust
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

The current local component passes 79 synthetic SDK functions, negative types,
installed-file equality and an installed public-consumer protocol with successful
recovery/replay/refusal/close and deliberate foreign-record failure. These are
protocol/process checks, not actual authority or cryptographic qualification.
The Ubuntu candidate uses all eleven real administrative command families,
independent Rust signed-journal comparisons, exact replay across a server restart,
changed-intent refusal, one original receipt and original resource drains. Its
runtime results remain pending. Ordinary-caller/current-revocation and uncertain
HTTP commit scenes also remain required. See [the evidence](../evidence/gate-6/node-administration-sdk.json).
