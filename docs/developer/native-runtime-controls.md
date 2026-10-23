# Native Connection hold, resume and mute

The production signed QUIC service accepts `connection.update` with `hold`,
`resume`, `mute` and `unmute` on the original physical peer that owns the
Connection. Retain `cid`, `sid`, `connid`, `operation_id` and the current
`expected_revision`. Each wire retry has a fresh signed envelope ID while keeping
the exact original logical intent. Current named `connection.update`, original
offer/Device/key, member epoch, endpoint and source/recipient authority remain
required. HTTP can inspect or cancel an operation; it cannot perform this native
control on a Connection.

| Action | Payload and behavior |
|---|---|
| `hold` | An optional `streams` list selects exact current logical Streams; omission selects all current bindings. Default hold lets the endpoint receive and stops its publication. `details.direction` may explicitly select endpoint `recvonly`, `sendonly`, or `inactive`. |
| `resume` | An optional exact `streams` list, or all current bindings. Clears hold and pause, preserving the originally negotiated directions and independent mute state. |
| `mute` | Required nonempty `streams` list. Stops the endpoint's publication on those Streams while preserving allowed receive media. |
| `unmute` | Required nonempty `streams` list. Clears source mute while preserving any hold or pause. |

For example, the public canonical client's `request_value` can submit the
following payload as `connection.update` with its saved command scope:

```json
{"action":"mute","streams":["microphone"],"app_note":{"reason":"user choice"}}
```

Optional extensions remain part of the immutable logical intent. Mixing these
controls with codec preferences or quality configuration is refused. Unknown
Streams, stale revisions and a different physical peer cannot acquire a control
effect. A held invocation must match the exact engine plan's action, stream set,
revision and Connection. It cannot borrow the initial offer's negotiation grant
or create a counteroffer.

One committed control advances the stream revision once and changes no codec,
handle, transport generation, Session role or allocated Connection count.
`operation.updated` plus its correlated ACK reports the retained operation
outcome. Inspect that outcome or current-authorized `operation.get`; an ACK alone
does not assert successful media control. A completed result contains the action,
Connection generation, new stream revision, `resources_changed:false` and
`gate_streams`, the actual returned per-Stream observations. Each observation
includes its binding, active/paused state, source mute and optional held direction.
Binding and held directions describe the server's perspective, so an endpoint
`sendonly` hold appears as server `recvonly`.

The bounded `readiness_json.last_gate` field retains the latest partial captured
observation for the same live original generation. It is not a complete current
Stream inventory or a media grant. The journal and operation ledger retain the
original historical outcomes. Replaying an old mute after a later unmute returns
the old receipt without muting the Connection again.

The engine drains previously admitted sends and changes the private routing
generation under current held authority. Fresh packet IO still reacquires current
media authority. A possible effect after an interrupted call remains uncertain
and cannot be blindly reissued. A known successful return is captured before
storage recovery: journal failure rolls back all durable outcome changes while
the runtime retains only that receipt, releases command readers, and retries
metadata recording without another Core call. Cancellation after that local
commit does not rewrite it as an unapplied control. Completion after retirement
records history without reviving the slot or a successor runtime.

The local production cases exercise EdDSA and ES256, actual decoded duplex G.711,
one-way hold in each direction, inactive hold, independent mute/resume, exact
logical replay, wrong-peer/stale/unknown refusal and storage recovery while an
independent role writer and cancellation proceed. Browser, Linux, remote/provider
profiles and complete Gate 6 acceptance retain their assigned verification.

An additional EdDSA composition verifies mute across a real PCMU-to-PCMA codec
replacement. The new physical binding remains muted while receive audio uses
the new codec. An explicit current unmute restores decoded publication; replaying
the old mute receipt preserves its old binding observation and leaves the newer
binding and state intact.
