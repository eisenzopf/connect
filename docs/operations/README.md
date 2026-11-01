# Current pre-alpha operations

This guide targets the current signed `connect/2` contract and schema **116**.
The CLI's outer result envelope uses `connect-control/1`; trusted provisioning
responses use `connect/2`. These identify the current layers of the interface.
It is an unpublished Gate 6 operations draft. Gate 6 acceptance, remote/media
qualification and publication remain separate. See the
[breaking-change draft](../releases/gate-6-draft.md),
[Ubuntu installation guide](../LINUX_INSTALL.md),
[canonical HTTP reference](../../contracts/openapi-canonical-http.json) and
[administrative HTTP reference](../developer/server-http.md).

Use one verified binary/package and its matching documentation. Preserve its
digest with backups. Unsupported protocol versions and database schemas refuse;
there is no old-client compatibility, historical converter or downgrade path.
An HTTP application facade and signed canonical control are explicit current
interfaces to the same service/store. An application config is not a native
config, and a control receipt cannot supply physical media custody.

## Exercise the commands on disposable state

The following paths are examples in an operator-owned working directory. Keep
them private and choose unused loopback ports. The request JSON files linked
below are public templates; copy them into this working directory. They contain
synthetic intent and caller-selected logical IDs, never credentials or internal
database edits. Run mutations only in this fresh fixture.

```sh
set -eu
umask 077
mkdir private
connect init --directory private/current \
  --bind 127.0.0.1:4433 --http-bind 127.0.0.1:8080
connect serve --config private/current/server.toml > private/server.log 2>&1 &
server_pid=$!
```

Wait for `ready transport=quic` and `profile=connect/2` in the private log. Then:

```sh
connect health --config private/current/operator.application.toml > private/health.json
connect request --config private/current/operator.toml --envelope < conversation-list.json
connect request --config private/current/operator.toml --envelope < conversation-create.json
connect request --config private/current/operator.toml --envelope < operation-get.json
```

Use these exact templates:
[list](requests/conversation-list.json), [create](requests/conversation-create.json),
[original status](requests/operation-get.json). The create ID is deliberately fixed
for this fixture. Replaying the unchanged create preserves one effect. A new
business action needs a separately saved intent and logical ID. Each CLI exchange
supplies a fresh signed transport envelope; it does not replace the logical ID.
The envelope contains `result.Ok` or `result.Err`; rejection also exits nonzero.
Stop on unexpected failure; do not continue against an older existing path.

The health result distinguishes configured/unconfigured readiness and exposes
global numeric observations only with current `health`, `admin`, distinct
`installation.observe` and unrestricted recipient authority. A participant's
health may omit operational diagnostics. Busy/retired probes are unavailable,
not zero usage. Check actual admission counters before interpreting overload.
The [G6-057 evidence](../evidence/gate-6/operational-visibility.json) names the
tested limits and resource-measurement scope. A running process or ready listener
does not establish remote reachability or working media.

## Reconcile the original action

Keep the saved logical ID and intent after a timeout, disconnect or lost reply.
Use `operation.get` with current authorization first. Read `operation.updated`
state/result and its causal observations. Do not invent a new ID because a reply
is missing. A completed original can be returned without another effect; current
revocation and recipient ceilings still apply on every read/replay.

For `unknown`, `interrupted` or `reconciliation-required`, preserve the original
resource/correlation and reservation. Use the configured authenticated provider
status path or an attributed runtime observation. Public reconciliation requires
existing evidence bound to that original operation and current generation;
operator assertions cannot manufacture remote termination or local completion.
A completed cancel/reconcile **decision receipt** does not necessarily mean its
target was cancelled: inspect its disposition and query the target. Do not redial,
rebridge or start another worker to guess the outcome. See
[crash recovery](../developer/crash-recovery.md) and
[operation authority](../developer/operation-authority.md). This guide's rehearsal
performs local control only; it makes no external calls.

## Rotate a scoped application credential

Keep the installation operator private. A client rotation means issuing a new
scoped credential and Device/key, proving its admission, switching that client,
and terminally revoking the old credential. It does not rotate TLS or the server
signing pin. Those have independent configuration, validity and trust procedures.

For this fixture, save the [identity intent](requests/identity.json) as
`identity.json`, then obtain its server-selected Identity ID:

```sh
connect provision --config private/current/operator.application.toml \
  < identity.json > private/identity-result.json
```

Create and retain `private/credential-a.json` and `private/credential-b.json`
using the [scoped issuance fields](../developer/credential-installation.md):
distinct saved `operation_id`, the returned `result.Ok.identity_id`, literal
`operations` and `recipients`, and a future absolute `expires_at` within the
issuer's lifetime. For this fixture use operations `health`, `list`, `send`,
`history`, `operation.get` and an empty recipient set. An empty set grants no
peer audience; it is not a wildcard. Neither credential is an administrator.

```sh
connect issue --config private/current/operator.application.toml \
  --native-template private/current/operator.toml \
  --request private/credential-a.json --output private/client-a
connect issue-resume --config private/current/operator.application.toml \
  --installation private/client-a
connect issue --config private/current/operator.application.toml \
  --native-template private/current/operator.toml \
  --request private/credential-b.json --output private/client-b
connect request --config private/client-b/client.toml --envelope < conversation-list.json
```

`issue` confirms actual signed admission before marking installation complete.
If delivery is uncertain, use the same `issue-resume` command and original
directory, issuer and endpoint. It preserves the original credential, Device,
key and absolute expiry. A partial/unsealed/changed installation refuses rather
than silently issuing another credential.

After the client has adopted B, read A's non-secret immutable `credential_id`
from its protected `issued.json`. Substitute that ID below; keep the revocation
ID and command before submission:

```sh
connect revoke --config private/current/operator.application.toml \
  --operation-id operations-guide-revoke-a-1 CREDENTIAL_A_ID
```

Repeat that exact command to recover uncertain delivery. Changing its target
under the same ID conflicts. Confirm A is refused and B still admits a signed
read. Expiry or revocation never extends on replay. A lost/expired operator cannot
regain authority by using a client credential; preserve state and use the separately
configured current administrative recovery procedure. See
[credential revocation](../developer/credential-revocation.md) and
[remote enrollment](../developer/remote-enrollment.md). Back up client-owned
credential installations separately; the server backup includes only the selected
operator/native recovery material, not every client's private Device key.

## Stop, back up and restore current state

Join the original server before maintenance. On the installed Linux service use
the [verified systemd procedure](../LINUX_INSTALL.md#stop-back-up-restore-upgrade).
For the owned foreground fixture:

```sh
kill -TERM "$server_pid"
wait "$server_pid"
connect backup --server-config private/current/server.toml \
  --config private/current/operator.toml --output private/before-change
connect restore --backup private/before-change --directory private/restored
```

Both output directories must be new. Backup excludes live writers, authenticates
the selected current administrator, captures one database/blob snapshot and
protects TLS/signing/native recovery material. The manifest binds source blob
size/digest to database metadata. A missing/corrupt/incomplete/unsupported backup
refuses before publishing restored state. Ambient environment secrets are absent;
reinject them independently. Preserve configuration and secret references.

Start only the restored copy, using its portable `server.toml` and `operator.toml`:

```sh
connect serve --config private/restored/server.toml > private/restored.log 2>&1 &
server_pid=$!
connect request --config private/restored/operator.toml --envelope < conversation-list.json
connect request --config private/restored/operator.toml --envelope < operation-get.json
```

Wait for readiness before the reads. Verify the original conversation and
operation result. Restore retains original IDs, absolute credential/pin expiry
and revoked state. It restores no physical connection or external call. The
portable backup contains the selected native operator config, not an HTTP
application config; do not assume an `operator.application.toml` was restored.
Never run the original and restored installation identity together on public
endpoints. Keep the original state and matching binary until verification.

## Explicit reset or unsupported-state handling

If a binary refuses an unsupported schema, stop. Preserve the complete original
state, configuration, secrets and matching binary in protected storage. Use that
binary's documented stopped backup if supported. Do not rewrite schema numbers,
remove database tables or point an older binary at newer state. There is no
automatic conversion, reset or rollback after a failed start.

For an intentional fresh start, stop the restored/original runtime and choose a
**different** destination. Preserve all previous directories:

```sh
kill -TERM "$server_pid"
wait "$server_pid"
connect init --directory private/reprovisioned \
  --bind 127.0.0.1:4433 --http-bind 127.0.0.1:8080
```

This creates a new installation/Identity/trust state. Reprovision clients and
endpoints deliberately. Old credentials and operation IDs do not become current
authority in it. `init` and `restore` refuse existing destinations without
overwriting them. Keep unresolved external effects associated with the preserved
old installation; fresh state does not prove they ended.

## Retention and cleanup

Configure the complete named policy in protected `server.toml`, then gracefully
restart. For example, this synthetic fixture policy retains metadata for 60 seconds:

```toml
[record_retention]
name = "operations-guide-metadata-60s-v1"
metadata = { kind = "after", seconds = 60 }
transcript = { kind = "until-deleted" }
recorded_media = { kind = "until-deleted" }
provider_reference = { kind = "until-deleted" }
```

Replace the existing retention table instead of adding duplicate TOML keys. The
rehearsal starts the actual server with this policy. The Session terminal
transaction freezes deadlines; changing configuration cannot extend an existing
record. The owned worker processes bounded batches and retries durable pending
decisions after restart. Expired projections lose access even while physical
cleanup is pending. Worker failure degrades readiness; resolve the recorded error
without deleting operation rows or manufacturing a completed cleanup.

The current producer records metadata. Transcript/media/provider-reference policy
does not imply those sources were captured. Local retention/erasure does not prove
provider deletion or physical disk sanitization. See the exact
[retention behavior and evidence](../developer/record-retention.md). General
operation tombstones retain original IDs/digests and prevent expired-ID replay;
deleting them is not an operator cleanup procedure. Retiring files or an entire
installation is an explicit owner action after stopped backup and resolution of
outstanding external effects. No guide command recursively deletes state.

## Verification

`python3 scripts/check-operations-guide.py` builds the normal production CLI and
exercises these forms on an isolated Ubuntu 24.04 x86_64 fixture, consuming the
exact linked JSON templates. It preserves private command inputs, replies, keys
and logs under ignored `.local/`; curated evidence contains stage outcomes,
source/binary/template hashes and no credentials. It also checks occupied-path,
live-writer, corrupt/unsupported-backup and revoked-client refusals, original-ID
one-effect recovery, restored revocation and preserved old state after a fresh
reprovision. Current command execution remains unqualified until its evidence
passes; earlier Linux installation, recovery and retention scopes are linked
separately. No model, provider, telephone or media execution is part of this lane.
