# Telephone endpoint replacement

The local canonical server can move one speaking connection to a provisioned
telephone Endpoint. The Conversation, Session, logical Participant and membership
epoch stay the same. The retained remote connection stays live. Telephone uses
an independently configured SIP account; it does not borrow a native Device key.

Configure the server before it accepts native peers or HTTP commands. Add this
section to the signed installation's `server.toml`, using the actual current
Store credential ID and verified Endpoint route reference:

```toml
[telephone]
bind = "127.0.0.1:5068"
media_port_start = 20000
media_port_end = 20127

[[telephone.routes]]
credential_id = "REPLACE_WITH_CURRENT_STORE_CREDENTIAL_ID"

[telephone.routes.policy]
route_reference = "config_callback"
sip_authority = "127.0.0.1:5099"
confirmation_digits = "55#"
timeout_seconds = 30
```

This is the existing local canonical profile: the SIP bind must be a concrete,
nonzero loopback address. The explicit RTP range must start at 1024 or higher
and end at or above its start. The route count is bounded by the connection
limit. Empty, duplicate, invalid or unavailable route accounts are refused.
Start it with the usual `connect serve --config .local/signed/server.toml`.
The server installs the SIP adapter on the same Core before either intake opens.
There is no public configuration or arbitrary telephone destination command.

The [trusted provisioning interface](trusted-provisioning-sdk.md) must create and
verify the telephone Endpoint and Device for the logical member. Its address is
E.164, its capability includes audio, its route owner identifies the independent
SIP account, and its `secret_ref` equals the configured `route_reference`. The
configured credential belongs to that route owner, permits `sip_dial` and covers
the logical member and retained recipient. These are current checks at each
phase and physical write; successful server startup does not preauthorize calls.
The initiating operator and both original native connections retain their own
current permissions. Endpoint verification alone does not establish readiness.

Optional trunk settings use the existing server SIP fields. Only the environment
variable's name is saved; the server resolves its password at startup:

```toml
[telephone.outbound]
proxy_uri = "sip:carrier.example;lr"
identity_uri = "sip:account@carrier.example"
auth_username = "account"
auth_password_env = "CONNECT_CALLBACK_SIP_PASSWORD"
# registrar_uri = "sip:carrier.example" # only when this trunk needs registration
```

This configuration interface is implemented; named carrier/PSTN rehearsal and
registration on this canonical composition remain separate qualification lanes.
Use only the designated bounded endpoints and configuration required by G6-093.
The default tests use isolated loopback peers and no provider credentials.

Submit `connection.replace` through the signed Rust client or explicit canonical
HTTP client. Preserve one `operation_id`; assert the current Conversation,
Session, source `connid` and topology revision. The payload selects the verified
`endpoint_id` and `retained_connid`, with no destination URI or caller-supplied
confirmation flag. See the [transition interface](canonical-transitions.md) and
[HTTP/OpenAPI reference](canonical-http.md).

The server records the exact candidate and Call-ID before its once-only INVITE.
Answer, actual private decoded duplex and the configured attributed digits are
separate requirements. Early/output-only, incorrect or retransmitted digits
cannot confirm. Private input stays outside the main audience. Until the actual
compound commit, the old speaking route stays active and the operation is still
preparing. Only a completed result establishes the new generation.

After commit, the new telephone connection has its own current credential,
Endpoint/Device, Session role, accepted codec and actual connection lifetime.
Revoking the old native credential or original operator, cancelling the completed
operation or reaching the old preparation deadline does not end that connection.
Its own authority loss ends only the new telephone route. Actual SIP absence is
required before durable retirement. The retained native peer remains intact.

Matching replay uses the original receipt and cannot redial. Unknown submission
requires reconciliation rather than another allocation. Shutdown cancels the
actual owned SIP work before releasing its listeners; startup failure and dropping
the server also own cleanup. After restart, an uncertain callback remains unknown
and replay cannot create another call.

A completed operation remains historical after restart. Its connection must be
retired or interrupted with current readiness cleared; the receipt cannot revive
that route. Server-owner drop also fences the old ready claim under the successor
epoch. Known captures and attempt IDs remain unchanged. Actual late old-dialog
INFO and RTP cannot confirm or publish into the new runtime. A discarded HTTP
command reply is recovered with the same operation ID and never another dial.
See [telephone restart evidence](../evidence/gate-6/telephone-restart.json).

[Telephone movement evidence](../evidence/gate-6/telephone-movement.json) records
actual PCMU/PCMA movement and independent lifetime on local macOS arm64. Installed
configuration, failure cleanup, ringing restart and answered refusal checks are
recorded in [configured telephone evidence](../evidence/gate-6/telephone-configured.json). The complete local movement requirement audit and final mixed-role/refresh checks are in
[telephone role and refresh evidence](../evidence/gate-6/telephone-roles.json).
G6-041 is complete for its recorded local server/native/SIP scope. Browser,
remote hosting, carrier/PSTN, performance and independent Gate6 acceptance remain
their assigned qualification tasks; publication remains separate.

After a completed move, native clients can subscribe to the actual public
telephone stream ID or retain a `from_participant` audio subscription across the
move. Participant intent follows the same logical membership and epoch. Every
materialized source still uses its current independent route account and media
permissions. A named stream unsubscribe overrides retained participant intent;
subscribing to that stream again clears the exclusion and restores delivery.
These controls require the receiver's own current subscription authority and do
not grant permission to dial or borrow the original endpoint's credentials.

An established telephone connection can be the source of another
`connection.replace` command. Select its current public connection ID, a distinct
verified native Endpoint, and the retained connection. The new device uses its
own signed client and the public `allocate_native_replacement_target` and
`begin_native_replacement_media` helpers. Actual decoded input and guarded output
are required before movement commits; an answered offer alone is insufficient.
The established telephone keeps its duplex route until that commit. The new
native connection advances the lineage generation and remains independent of
the old SIP credential. Replaying either movement preserves its original receipt
without another telephone call.


Session role changes apply to every current physical native and telephone source
of the same logical participant. Observer suppresses publication while permitted
receive remains available; moving to a new native endpoint preserves that role.
Established SIP refreshes use the confirmed dialog and current route authority,
independently of the original preparation/operator. A rejected refresh completes
its SIP transaction and keeps the existing media route when its authority remains
current. Assistant context delivery is tracked separately in G6-042.
