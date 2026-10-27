# Explicit browser ICE services

`BrowserVoiceSession.open` and `BrowserVideoSession.open` accept an optional
`iceServers` list. Pass designated STUN/TURN services for the deployment. Omission
retains native host candidates; the SDK does not select a public service.

```typescript
const session = await BrowserVideoSession.open(client, {
    ...originalConnection,
    inputStream,
    iceServers: [{urls: "stun:stun.example:3478"}],
});
```

The example address is illustrative. TURN entries accept a string `username` and
`credential` alongside their `turn:`/`turns:` URLs. Keep these credentials in
application-owned private configuration. The SDK snapshots the list before
capture permission can yield, gives it only to the original native RTC peer and
keeps it out of logical Connection intent payloads and saved operation IDs.

Configuration is bounded to8entries,4URLs per entry and16URLs total. Each URL
is at most2048characters, usernames512 and credentials2048. Supported schemes
are `stun`, `stuns`, `turn` and `turns`; TURN may explicitly select UDP or TCP.
Ports must be1–65535. TURN needs nonempty username/credential strings. Invalid
configuration fails before capture or signaling, with an error that omits private
values. Capture, original signaling and ICE gathering share the existing deadline.

The canonical example copies `connection.iceServers` from its private signed media
configuration into these options. The isolated Firefox fixture uses an owned
loopback STUN responder and an explicit Firefox loopback ICE test preference;
normal certificate verification and address obfuscation remain enabled. This
fixture configuration does not qualify remote or default Firefox connectivity.
See [browser ICE service evidence](../evidence/gate-6/browser-ice-services.json)
for the actual run outcome and remaining work.
