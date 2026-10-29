# Owned Node incoming invitations

G6-062 implementation is complete in its [recorded scope](../evidence/gate-6/node-implementation.json). Full installed package/platform parity and remaining runtime failures are tracked in G6-069/G6-070/G6-096/G6-098. Earlier evidence below retains its original scope.

`client.incomingInvitations(options)` captures the approved finite timeout and
signal and returns an async iterator. Each `next()` uses those captured options;
`for await` consumes original invitation metadata. The required `from` field
identifies the inviter. Additional metadata includes the scoped `Session`, target
participant (`to`), original invitation operation/source IDs, medium, intent and
immutable capabilities. Metadata grants no native/media or decision authority.

```ts
const options = { timeoutMs: 4000 };
const incoming = client.incomingInvitations(options);
try {
  for await (const hint of incoming) {
    const current = await hint.session.refresh(options);
    await (await hint.session.reject({
      reasonCode: 486, reason: 'Unavailable',
      expectedTopologyRevision: current.topologyRevision,
    }, { ...options, operationId: savedDecisionId })).wait(options);
    break;
  }
} finally {
  await incoming.close();
}
```

Loop break or application failure detaches the reader through its iterator
`return()`; explicit `close()` is idempotent. Close uses bounded cleanup even if
the captured reader signal has been cancelled. Control stays usable. A closed
iterator returns `{done:true,value:undefined}`. A deadline consumes no delivery;
a cancelled reader can close and a fresh reader can reclaim unread metadata.

One original helper owns the Rust `InvitationStream`. One reader claims the
inbox, and one pending hint stays bounded to 63 KiB until its exact owner
acknowledges the delivery. Hint/source IDs remain distinct from a separately
saved decision ID. Hints can be stale after Session cancellation or membership
removal; refresh before deciding under current authority.

Rust polls the original SDK future for at most 25 ms without consuming a dropped
wait. An abandoned Node reply is re-polled through Rust's current closure/expiry
and inbox-failure checks before return. No replacement helper or mutation replay
occurs. Participant helper 3 and the matching CLI are required; administrative
helper 2 and local validators 1 remain separate.

All 94 component/helper/HTTP/type functions pass with 217 guarded inputs unchanged;
all prior 93 checks remain. Nine incoming functions also pass through the installed
public package with exact distribution equality. Negative declarations require
factory options, reject the earlier per-wait API and preserve source/identity
and metadata ownership types. The earlier draft interface is superseded; the
approved contract and outline were left unchanged. Current real iterator/native
incoming media/decision/revocation/reconnect and graceful drain verification,
durable events, remote HTTP and full G6-062 remain open. See
[the contract evidence](../evidence/gate-6/node-incoming-contract.json).

`operationId` is the server-issued invitation-source correlation. The caller's
private mutation ID and your separately saved decision ID retain their separate
meanings. The historical d85 native fixture incorrectly compared source and
caller IDs; both runs failed at that equality. The corrected fixture compares a
digest of the received source with the authoritative host row and requires a
distinct caller ID. Production metadata is unchanged. Three real reader-state
units passed in d85, while complete incoming execution remained failed. See
[the retained runtime](../evidence/gate-6/node-native-d85.json) and
[the source comparison](../evidence/gate-6/node-incoming-source.json).
