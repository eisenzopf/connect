# Current canonical JSON CLI

Use the explicitly installed signed native configuration for canonical control.
The CLI sends through the public canonical client with its configured Identity,
Device keys and trust. Missing MCP tools do not authorize an administrator key
or a different credential. Keep the same access mode during recovery.

```sh
connect request --config "$connect_native_config" --envelope < send.json
```

`send.json` is a public `Command`, with the canonical payload and separate scope:

```json
{
  "command": {
    "type": "message.send",
    "payload": {
      "msg_id": "saved-message-id",
      "to": "all",
      "content_type": "text/plain",
      "body": "Here is the review update.",
      "body_encoding": "utf8",
      "label": "connect-messages",
      "attachments": [],
      "in_reply_to_msg": null
    }
  },
  "scope": {
    "conversation_id": "EXISTING_CONVERSATION_ID",
    "operation_id": "saved-send-operation"
  }
}
```

Save the entire intent, message audience, logical operation ID and Message ID
before IO. The client supplies fresh physical envelope IDs/timestamps/signatures.
A new transport attempt never needs a new logical mutation. After uncertainty,
query the original operation with its exact scope:

```json
{
  "command": {
    "type": "operation.get",
    "payload": { "operation_id": "saved-send-operation" }
  },
  "scope": { "conversation_id": "EXISTING_CONVERSATION_ID" }
}
```

Use original Session/Connection scope when the original operation had it.
Finite journal reads use `conversation.subscribe` with `payload.live:false`.
The initial read returns a snapshot/cursor; resume with `payload.after`.
An ACK is a bounded control reply, not decoded media or external delivery.

Named application APIs, including optional Task workflows, are a separate
explicit HTTP configuration and command mode. They cannot replace canonical
communication on failure. For an authorized ordinary Task owner:

```sh
connect request --application --config "$connect_application_config" \
  --task-id "$connect_task_id" --envelope < task-get.json
```

```json
{
  "op": "task",
  "request": {
    "task_profile": "connect-task/1",
    "command": { "action": "get", "task_id": "SAVED_TASK_ID" }
  }
}
```

The Task ID is a work reference, not a credential. Use the installed narrow MCP
adapter for a protected HostSession/provider grant; this example does not turn
it into a broader native or HTTP identity. Follow the current Task schema for
wait cursors, attributed replies and explicit consumption. A tracked follow-up
uses the received journal reply event; the separate Reply command uses the
original question Message ID.

Input is bounded to64KiB. With `--envelope`, stdout preserves the
`connect-control/1` structured response and a refusal also exits unsuccessfully.
Check both. A canonical success contains `kind:canonical_reply` with the checked
current reply frames; retain original logical IDs from the saved intent.
The MCP tool's root resource fields and the CLI's nested `scope` are different
shapes. Never mechanically send MCP arguments or the retired generic `op:send`
shape to the canonical CLI.
