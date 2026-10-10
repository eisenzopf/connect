# Native JSON CLI fallback

This is the current developer/runner interface. Use it only with the protected,
narrow client configuration and task/address supplied for that workflow. A
Connect MCP invocation grant is not currently a native CLI credential. Missing
MCP tools do not authorize exporting an owner/operator token to a model.

The installed binary accepts one public operation on stdin:

```sh
connect request --config "$connect_client_config" \
  --task-address "$connect_task_address" --envelope < task-get.json
```

`connect_client_config` names the developer-provided protected client config;
`connect_task_address` is a permitted address used for task capability discovery.
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
Old servers that do not advertise `connect-task/1` are explicitly unsupported.

An MCP communication call derives its task/conversation server-side. A native
`invoke` command instead contains the protected task ID, step/request IDs and a
typed communication operation. Do not mechanically send MCP arguments to the
CLI, invent fields or introduce a new conversation during recovery. Current
schemas and public SDK types define the command shapes.
