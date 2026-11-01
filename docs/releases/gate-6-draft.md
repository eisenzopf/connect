# Gate 6 breaking-change draft

This is an unpublished pre-alpha handoff draft, not a release announcement.
The current candidate uses `connect/2`, schema **116**, and the frozen package
names/version recorded in the [SDK candidate evidence](../evidence/gate-6/sdk-candidates-verified.json).
The generic CLI result wrapper is `connect-control/1`; native signing and trusted
provisioning retain their separately declared `connect/2` profiles.
Gate 6 acceptance and owner release approval remain open.

- Signed canonical commands use enrolled Identity/Device/key bindings and pinned
  server signing trust. Ordinary SDK, HTTP and protocol responsibilities share
  the current service/store; unsupported profiles refuse explicitly.
- Mutations preserve caller-owned logical operation IDs across uncertain delivery.
  Current authorization applies before replay. Journal state is authoritative;
  notifications and local app progress are hints. Stored completion cannot revive
  a physical peer or manufacture provider settlement.
- Scoped `issue`/`issue-resume` installation replaces the old generic single-file
  issuance interface. Trusted provisioning and revocation use explicit protected
  HTTP configuration, current administration and immutable operation IDs. Browser
  clients receive only their needed capabilities.
- Current backup/restore captures one stopped database/blob snapshot with selected
  TLS/signing/native recovery assets. It refuses unsafe, incomplete, corrupt,
  unsupported or occupied destinations. Ambient secrets and every client-owned
  private Device key are not exported by a server backup.
- Existing historical state is preserved with its matching binary. The current
  runtime does not migrate historical schemas, support old clients, downgrade
  state, silently reset data or automatically redial an uncertain external effect.
  Deliberate reset initializes a new path and requires explicit reprovisioning.
- Operational diagnostics enforce separate global observation authority, expose
  actual resource owners and explicit unavailable observations, and keep admission,
  idle expiry and drain bounded. Resource samples are scoped measurements, not a
  remote deployment or capacity/performance claim.

Use the [operations guide](../operations/README.md) for current commands and
failure/recovery procedures. G6-054 media qualification remains deferred. Installed
reference-app control flows pass on Node 22/Linux and macOS; negotiated browser
capture remains unfinished. The final transport/SDK matrices, integrated journey,
independent reviews, remote/live rehearsals and release sealing are still separate
required Gate 6 tasks. Publication is not authorized by this draft.
