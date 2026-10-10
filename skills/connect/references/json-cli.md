# Native JSON CLI fallback

Use the developer-supplied protected **Native task-grant** configuration and
bound task ID. HostSession/provider MCP grants are not native credentials.
Missing tools do not authorize exporting an owner/operator token to a model.
Developer setup uses `task-access export-native`; the model does not mint,
refresh or select another grant.

The installed binary accepts one public operation on stdin:

```sh
connect request --config "$connect_client_config" \
  --task-id "$connect_task_id" --envelope < task-get.json
```

`connect_client_config` names the protected Native grant configuration;
`connect_task_id` is the task supplied by installation. The CLI negotiates it
through a matching Get, without directory permission.
Neither is inferred from message text. The JSON input is limited to 64 KiB.
The native helper uses UCTP/QUIC; an HTTP control option is a separate interface.

For inspection, `task-get.json` has this shape; replace `TASK_ID` with the task
supplied by protected setup:

```json
{
  "op": "task",
  "request": {
    "task_profile": "connect-task/1",
    "command": { "action": "get", "task_id": "TASK_ID" }
  }
}
```

For a bounded journal read, preserve the previously returned `next` value as
`after` (initially null):

```json
{
  "op": "task",
  "request": {
    "task_profile": "connect-task/1",
    "command": {
      "action": "wait", "task_id": "TASK_ID",
      "after": null, "limit": 100, "wait_ms": 1000
    }
  }
}
```

With `--envelope`, stdout preserves the `connect-control/1` response and its
structured success/error; a rejected request also exits unsuccessfully. Inspect
the response and process exit. An empty inbox remains pending. Transport failure
can leave a mutation uncertain, so retain and reconcile its original logical IDs.
An unsupported task profile never negotiates or silently falls back to owner authority.

An MCP communication call derives its task/conversation server-side. A native
`invoke` command instead contains the protected task ID, step/request IDs and a
typed communication operation. Do not mechanically send MCP arguments to the
CLI, invent fields or introduce a new conversation during recovery. Current
schemas and public SDK types define the command shapes.
