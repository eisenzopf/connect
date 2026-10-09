# Carry a Telnyx telephone call through Connect

This operator runbook prepares the Gate 3 route:

```text
handset ↔ Telnyx PSTN/SIP ↔ Connect SIP/RTP ↔ Vapi duplex WebSocket
```

This differs from Gate 2's direct Telnyx/Vapi number. A call answered while the
Connect media bridge is stopped is evidence of the direct route, not this route.

## Preserve the current route

Before changing a number, use the Telnyx API to read its `connection_id`, the
candidate credential connection, and its outbound voice profile. Save the full
responses in an ignored, owner-only `.local/` directory. The restoration action
is to update the number with the exact saved `connection_id`; never infer it from
a display name. Provider readback is setup evidence and does not qualify audio.

The Connect credential connection must be active, use a profile that permits
the test destination, assert an owned caller ID, and include `G711U` in its
inbound codecs. Keep its generated username and password private.

## Configure Connect

SIP and RTP require publicly reachable UDP addresses and ports. An HTTPS tunnel
cannot carry this traffic. In a private `server.toml`, configure the listener,
the explicit durable route, and the carrier trunk:

```toml
[sip]
bind = "0.0.0.0:5060"
media_bind_ip = "PRIVATE_INTERFACE_IP"
advertised = "PUBLIC_IP:5060"
media_public = "PUBLIC_IP:0"

[sip.outbound]
proxy_uri = "sip:sip.telnyx.com;lr"
identity_uri = "sip:+1OWNED_NUMBER@sip.telnyx.com"
auth_username = "TELNYX_CONNECTION_USERNAME"
auth_password_env = "CONNECT_SIP_PASSWORD"

[sip.route]
credential_id = "ROUTE_CREDENTIAL_ID"
conversation_id = "CONVERSATION_ID"
assistant_identity_id = "VAPI_IDENTITY_ID"
session_id = "ACCEPTED_AUDIO_SESSION_ID"
```

Export the password only in the server process environment. Startup fails if it
is absent or empty. Connect applies the identity to both From and
P-Asserted-Identity and sends originated dialogs through the configured loose
route. SDK clients supply a SIP destination URI, never trunk credentials.
`media_bind_ip` is the concrete local interface used to allocate RTP; it must
not be the public NAT address unless that address is actually assigned locally.
A zero public media port preserves rvoip's per-session local RTP port in SDP.

## Qualify and restore

Run local SIP interoperability first. For the controlled live test, record the
saved direct route, Connect process revision, conversation/session/connection
IDs, and both provider call references. Announce the owned caller ID before an
outbound call. Confirm intelligible audio in both directions, at least three
turns, context use, interruption, and teardown initiated from each side.

For inbound testing, temporarily assign the owned number to the prepared
Connect route only while the public SIP/RTP listener is running. After the test,
restore the exact saved number `connection_id`, read the number back, and stop
the public listener. Keep the redacted result under `docs/evidence/gate-3/`; keep
account IDs, credentials, raw SIP, phone numbers, and provider payloads under
ignored `.local/` evidence.
