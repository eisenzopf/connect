# Install a local task grant

Developer setup creates a task through the public SDK or JSON CLI under a
currently authorized application credential. Use that credential to issue a
narrow grant for the task; give the model only the resulting protected host
connection. The model tool catalog does not expose issuance or refresh.

Enable an independent local MCP listener in the server configuration:

```toml
[mcp]
bind = "127.0.0.1:8193"
```

Start the application HTTP facade separately from any provider callback ingress:

```sh
connect serve --config .local/install/server.toml --http-bind 127.0.0.1:8194
```

Check `connect serve --help` for the current command options. Keep all backing
credentials and grant exports under your protected `.local/` installation.
Create `issue.json` with an existing task ID, a stable installation request ID,
concrete host-session identity, an allowed operation subset and an absolute Unix
expiry no more than five minutes ahead:

```json
{
  "profile": "connect-task-access/1",
  "command": {
    "action": "issue",
    "request_id": "install-review-task-1",
    "definition": {
      "task_id": "REPLACE_WITH_EXISTING_TASK_ID",
      "binding": {
        "kind": "host_session",
        "adapter": "connect-local-host",
        "external_id": "review-session-1"
      },
      "operations": ["history", "send"],
      "expires_at": 0
    }
  }
}
```

Replace the task ID and expiry before issuing. Both operations must be inside
the task's own scope; the task must belong to your authenticated principal.

```sh
connect task-access issue --config .local/install/developer.toml \
  --http-address 127.0.0.1:8194 --input .local/issue.json \
  --output .local/review-grant.json
```

The CLI saves pending intent first, then exports the credential with private
file permissions. It never prints the bearer. If delivery is interrupted, use:

```sh
connect task-access resume --config .local/install/developer.toml \
  --http-address 127.0.0.1:8194 --grant-file .local/review-grant.json
```

Resume preserves the original ID and expiry. An already-issued file is simply
reported as saved, without a fresh authority check or renewal. Use a new request
ID and output file for explicit refresh. Revocation uses the non-secret grant ID:

```sh
connect task-access revoke --config .local/install/developer.toml \
  --http-address 127.0.0.1:8194 --grant-id REPLACE_WITH_GRANT_ID
```

The exported bearer connects its installed task to `/mcp` on the MCP port.
Another task has its own grant; revoking one does not change another's authority.
The actual host installer and continuation journey remain tracked in the
[Gate 4 plan](connect-gate-4-implementation.html).

## Native SDK and JSON CLI

For native SDK use, issue a binding with `"kind": "native"`. HostSession and
provider grants cannot be converted into native SDK authority. Export the issued
file into a fresh private client configuration:

```sh
connect task-access export-native --config .local/install/developer.toml \
  --grant-file .local/native-grant.json --output .local/model.toml
connect request --config .local/model.toml --task-id TASK_ID \
  --envelope < .local/task-request.json
```

Export replaces the parent token with the Native bearer, retains the local endpoint,
trust material and identity metadata, and preserves existing output files. This
local conversion never contacts the server, refreshes a grant or proves it is current.
The server checks expiry, revocation, task and parent authority on authentication,
each task command and replay. Keep the grant file and exported config private.

Rust can build the same intended protected configuration with
`ClientConfig::with_task_grant(&protected_grant)`. Give Python/TypeScript NativeClient
that configuration path; explicit HTTP clients receive only the Native bearer.
Call `negotiate_task(task_id)` / `negotiateTask(taskId)` before task commands; this
reads the exact bound task without requiring directory/resolve permission.

Native grants enter the task facade: use `task(Invoke { ... })` for permitted
communication operations. An unwrapped `send`, history read or administrator
operation cannot borrow the parent's permissions. Keep logical mutation IDs and
inputs across reconnect/refresh; replay cannot revive cancelled or uncertain work.
Raw native media attachment and HTTP artifact routes are not inherited from the
parent credential and remain separate qualification work.

[Protected exchange contract](../contracts/connect-task-access-v1.md) ·
[Task contract](../contracts/connect-task-v1.md)
