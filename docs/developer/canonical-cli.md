# Canonical CLI control

`connect request --config <signed-native.toml> --envelope` accepts one public
canonical `Command` on stdin. `connect stdio --config <signed-native.toml>`
accepts sequential JSON lines with `id`, `command`, and optional `scope`.
Both use the public signed canonical native client and return checked current
frames under `kind:canonical_reply`. An exchange ID correlates the helper line;
`scope.operation_id` remains the separately persisted logical mutation ID.

[Exact command examples](../../skills/connect/references/json-cli.md) cover
Message send, original status recovery, finite journal pages and Task bootstrap.
The exported minimal MCP `agent.toml` is a scoped bearer asset. Native CLI control
needs its separately configured Identity, Device signing key and server trust.

## Bounds and recovery

Input is at most64KiB; each stdio reply is at most2MiB. Stdio admits one exchange
at a time and rejects unknown fields or the retired generic communication shape.
Invalid JSON/schema/framing terminates the helper without echoing its input.
Canonical event reads require explicit `live:false`; native live readers belong
to the public owned SDK interface.

A startup refusal is reported against each admitted exchange, and changing the
configuration file does not repair a running helper. Start a fresh helper after
explicitly fixing its configuration. Transport/protocol uncertainty fences all
later exchanges on that helper. Keep the entire original mutation intent and
resources; use a fresh helper to query its original operation. Nothing changes
credentials, API mode or transport on failure. EOF drains the owned native peer.
Current server authorization still applies to explicit mutation replay.

One-shot `--envelope` returns the structured error and exits unsuccessfully on
refusal. Stdio correlates each structured error and continues for ordinary
application refusals; transport/protocol failures remain fenced until restart.
Neither checked ACK nor completed control metadata proves decoded audio or
provider delivery.

## Named application and Task APIs

Select `--application` with an explicit `ApplicationClientConfig` for named
HTTP APIs. One-shot calls use `--task-id` or `--task-address` for Task negotiation.
Stdio lines contain `id`, `operation`, and optional `task_binding`. A bound
bootstrap must be an exact Task Get with the same Task ID and current profile.
Other Task calls require successful current Task negotiation. A narrow native
grant keeps its exact Task and permission ceiling; editing its configuration
with an owner's credential cannot promote an existing helper.

Ordinary create/send/session communication uses canonical commands. Named
application mode refuses those retired generic operations rather than dispatching
them through a different API. Historical Python/TypeScript generic NativeClient
wrappers need their approved SDK migrations; they do not consume this new
canonical JSON-lines shape.

Local behavior evidence is `docs/evidence/gate-6/canonical-cli-stdio.json`.
Scoped issuance and native enrollment use the
[saved-intent credential installer](credential-installation.md), with explicit
`issue-resume` after uncertainty. Retained inspection uses the
[application diagnostics](application-sdk.md#retained-task-diagnostics) config.
G6-065 remains in progress. Current CLI/HTTP/MCP, scoped issuance/recovery and
OpenAPI checks are recorded in the [facade audit](../evidence/gate-6/facade-scope-audit.json).
The actual supported-host rehearsal passed for its recorded candidate; a later
native private-audio probe failure remains unresolved. Remote, provider, Linux,
distribution and independent acceptance profiles need their own qualification.
