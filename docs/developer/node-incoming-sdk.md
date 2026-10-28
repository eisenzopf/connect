# Owned Node incoming invitations

`client.incomingInvitations()` creates a lazy local reader. `next(options)` returns
signed invitation metadata from the original Rust SDK: a scoped `Session`, the
original invitation operation/source IDs, sender/recipient, medium, intent and
immutable capabilities. It returns `undefined` after close. Metadata grants no
native transport, media custody or decision authority.

```ts
const incoming = client.incomingInvitations();
const options = { timeoutMs: 4000 };
try {
  const hint = await incoming.next(options);
  if (hint) {
    const current = await hint.session.refresh(options);
    // Use a separately persisted decision ID and the current topology revision.
    await (await hint.session.reject({
      reasonCode: 486, reason: 'Unavailable',
      expectedTopologyRevision: current.topologyRevision,
    }, { ...options, operationId: savedDecisionId })).wait(options);
  }
} finally {
  await incoming.close(options);
}
```

One original helper owns the Rust `InvitationStream`. Only one reader can claim
the inbox. A pending hint stays bounded to 63 KiB until its exact owner
acknowledges the delivery. Detaching retains unread metadata for the next local
reader. Hint/source IDs remain distinct from a separately saved decision ID.
Delayed hints can describe a cancelled Session; always refresh before deciding.

The helper polls the original SDK future for at most 25 ms. Dropping a wait
consumes no hint. Node cancellation/deadline detaches only that local wait,
leaving control usable and the original poll bounded. The next explicit wait
revalidates the retained hint through Rust, including current closure/expiry and
inbox failure, rather than returning an abandoned Node cache. One captured
caller deadline includes all polls/queue time; no replacement helper or mutation
retry occurs. Closing the reader leaves the client and its other controls live.

The participant adapter requires `connect-sdk-helper/3` and the matching CLI.
Administrative helper2 and local validators1 retain their separate profiles.
Incoming support is an interface capability, never a current grant/readiness.

All 90 component/helper/HTTP/type functions pass with 217 guarded inputs unchanged;
all previous 82 remain. Eight incoming functions also pass through the installed
public package with exact distribution equality. The native `node-controls/6`
candidate receives actual unsolicited voice+video metadata on the independent
peer and accepts using its hinted Session. Three Rust reader/ACK/inbox-health
units are required by Linux CI. Current compilation and actual native behavior
remain pending. Remaining media/decisions/revocation/reconnect, durable events,
explicit remote HTTP and full G6-062 qualification remain open. See
[the evidence](../evidence/gate-6/node-incoming-sdk.json).
