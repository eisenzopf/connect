# Native JSON CLI fallback

Use the installed protected agent client configuration for ordinary communication.
The same identity credential covers its authorized conversations and configured
operations. It needs no per-task grant, and a task ID is a work reference rather
than a credential. Missing tools do not authorize using an administrator key.

The installed binary accepts one public operation on standard input:

```sh
connect request --config "$connect_client_config" --envelope < send.json
```

For an ordinary message, `send.json` uses native fields:

```json
{
  "op": "send",
  "request_id": "saved-logical-message-id",
  "conversation_id": "EXISTING_CONVERSATION_ID",
  "body": "Here is the review update."
}
```

Retain the ID and exact inputs before uncertain dispatch. Normal sends have no
task `reply_to` or `wait_for_reply` field. Source references belong in the body.
Use request status and history/inbox to inspect receipt, delivery and replies.
Starting or ending a media leg does not create a replacement conversation.

For an optional tracked task, negotiate the same saved task reference:

```sh
connect request --config "$connect_client_config" \
  --task-id "$connect_task_id" --envelope < task-get.json
```

`task-get.json` has the public task shape:

```json
{
  "op": "task",
  "request": {
    "task_profile": "connect-task/1",
    "command": { "action": "get", "task_id": "SAVED_TASK_ID" }
  }
}
```

Task waits retain the returned `next` cursor as `after`, initially null:

```json
{
  "op": "task",
  "request": {
    "task_profile": "connect-task/1",
    "command": {
      "action": "wait", "task_id": "SAVED_TASK_ID",
      "after": null, "limit": 100, "wait_ms": 1000
    }
  }
}
```

An explicitly installed Native task grant also uses this facade, but its backing
identity's broader rights are unavailable. HostSession/provider MCP grants cannot
be used as native credentials. Preserve the access mode of existing installations;
the CLI never broadens a grant while retrying or resuming.

With `--envelope`, stdout preserves the `connect-control/1` structured response;
a rejection also exits unsuccessfully. Inspect both. The input is bounded to
64 KiB. The native helper uses UCTP/QUIC; HTTP control is a separate interface.

MCP and native task commands have distinct argument shapes. Use current schemas
and SDK types. Task-tracked Invoke follow-ups refer to the received journal event;
the public Reply command refers to the original question message. Do not substitute
source IDs or mechanically send tool arguments to the CLI.
