# Browser transport selection and recovery (G6-052)

`BrowserTransportClient` owns a browser's authenticated physical control
connection. It exposes the preferred transport, selected path, fallback reason
and local physical lifetime counter through `status`. The path labels are
`webtransport` and `wss-webrtc`; a selected path does not establish media
readiness. The low-level `BrowserClient` remains available through `client`.

```ts
import { BrowserTransportClient } from "@rudeless/connect/browser";

const connection = await BrowserTransportClient.connect({
    identityId: enrolledIdentityId,
    deviceId: enrolledDeviceId,
    endpoints: {
        webtransport: "https://connect.example/connect",
        "wss-webrtc": "wss://connect.example/connect",
    },
    preferredTransport: "webtransport",
    allowFallback: true,
    timeoutMs: 10000,
    attemptTimeoutMs: 3000,
    resolveEnrollment: async ({ signal, transport }) => {
        // Trusted application enrollment flow: scoped bearer, enrolled Device,
        // private signing CryptoKey and current server verification trust.
        return await loadCurrentBrowserEnrollment({ signal, transport });
    },
});
console.log(connection.status.selectedTransport);
```

Enrollment resolution and both possible physical attempts share one original
deadline. Each usable contact resolves its own current enrollment with an explicit
`transport` (`webtransport` or `wss-webrtc`) before opening. Unsupported or absent
contacts are skipped before resolving. Each result is captured for that physical
attempt. A fallback must keep the same Identity/Device; its approved credential
and key may be different. A missing preferred endpoint/API or a physical
opening failure can select the other configured secure contact. Authentication
refusal, signature failure, invalid configuration and authentication-stage
timeout terminate the round. Browsers can hide the cause of a physical network
failure; the SDK cannot classify every opaque browser TLS/network rejection.
The alternate contact requires its independently configured TLS and enrolled
server signing trust. Resolver failure or uncertainty ends the round; it cannot
trigger another credential or extend the deadline. Development certificate hashes apply only to WT.

There is no automatic reconnection, mutation queue or mutation replay. A lost
physical connection clears the selected path. Explicit `reconnect()` first
closes the old peer, resolves fresh enrollment for each usable contact and
authenticates again. Actor
and Device IDs must match the owner. A concurrent reconnect is refused; closing
the owner cancels pending enrollment/opening and prevents late adoption.

Save a logical operation reference before submitting a mutation. After an
uncertain reply, reconnect explicitly and observe the original operation:

```ts
await connection.reconnect({ signal: controller.signal });
const operation = await connection.recoverOperation({
    operationId: savedOperationId,
    command: "session.handoff",
    scope: {
        conversationId: savedConversationId,
        sessionId: savedSessionId,
        connectionId: savedConnectionId,
    },
});
// Keep preparing/interrupted/unknown visible; status reads never resend handoff.
```

`command` is the original canonical ledger kind. For example, a Conversation
creation is `create`, an unscoped Conversation message is `send`, and a Session
handoff is `session.handoff`. Supply the original command's asserted scope.
`recoverOperation` checks the returned original ID, kind, scope, state and
revision. The server applies current authorization; reconnect grants no access
to a previously visible operation after permission is withdrawn. Malformed or
ambiguous status responses close the physical peer.

`recoverConversation({ conversationId, cursor, limit })` reads one finite
journal page with `live:false`. It returns authoritative `conversation.event`
frames and summary metadata, excluding advisory notifications. With no cursor
it reads one snapshot page; pass the returned `nextPage` as `page` to continue.
Persist the returned cursor/page with the application's recovery state. An
explicit resync response obtains a fresh authorized snapshot. An expired page
restarts at the first snapshot page only for the exact signed
`409/state/resync_required` error, once, within the original read deadline.
Other refusals propagate. Limits are 1–63 items, default 32. A read started on a
retired physical lifetime cannot populate the new one.

The physical counter is a local stale-response fence. QUIC path migration can
retain the same physical connection. A new authenticated connection retires
old capture/media owners; applications must explicitly establish and observe
new media. Replacing a Device/Connection uses its distinct authorized operation.
Neither control reconnect nor selected path promises uninterrupted media after
a server crash. High-level SDK resource composition remains G6-062 work.

The signed WebCrypto component fixture passes all 379 SDK checks, including
52 new owner cases; see [transport owner evidence](../evidence/gate-6/transport-reconnect-owner.json).
Four installed Chrome/Firefox × EdDSA/ES256 control functions also pass with
normal private-CA TLS. They select WSS after actual UDP blackholes, recover
original operation records and cursor events over explicit reconnects, reject
an invalid bearer and recover with valid enrollment. The transition fixture
uses an accepted, unprepared consult and exact unready handoff/replacement
refusals; see [browser control evidence](../evidence/gate-6/transport-reconnect-browser-control.json).
The separate [service fault cohort](../evidence/gate-6/transport-reconnect-faults.json)
also passes all four browser/signature combinations. A bounded loopback WSS
proxy forwards canonical signed bytes unchanged and withholds one accepted
Message reply. After timeout, the owner recovers its original completed record
by status reads, with exactly one Message/Operation and one journal fact per
original recipient. The fixture force-cuts established WSS, refuses upgrades,
and blackholes packets on an established WT path. Explicit recovery selects
WSS, then explicitly restores WT. WSS uses normal private-CA trust; this WT
lane uses the designated short-lived leaf SHA256 pin plus independent enrolled
server-signature verification. Public/remote PKI qualification remains separate.

Separately issued and bound scoped credentials positively authenticate/read
before real 45-second expiry and production HTTP revocation. Subsequent reads
and reconnect fail; valid current enrollment restores access. Operator authority
and TLS private files remain in the Node fixture host. Browser, Node, proxy,
service and physical routes/slots are joined in every passing lane. Failed
initial candidates and the storage-full attempt remain recorded as failures.

The ignored `canonical_{chrome,firefox}_transport_owner_faults_{eddsa,es256}`
tests live in `gate6_canonical_webtransport`. They require the explicit installed
browser/Node/module paths used by the browser fixture, isolated private output
and a `TMPDIR` inside this checkout's ignored `.local/`; run one lane at a time.
Both private Playwright and Puppeteer dependencies must be prepared, including
the latter's `ws` module for the fault proxy. These are local synthetic tests;
they use no model, provider, telephone or remote account.

The [live controller checkpoint](../evidence/gate-6/live-controller-recovery.json)
passes both signing algorithms using official MCP control and separate public
native QUIC media owners. Each active private consultation and admitted pending
handoff survives three closed/joined controller reopenings. A test-only hold on
the existing role coordinator keeps the handoff pending while private native
packet IO continues. The original record, expiry and precondition stay fixed;
release completes the same handoff once, retaining the remote leg and retiring
the outgoing leg. All 18 marked PCMU wire windows pass the existing audience
and liveness checks. Waveform fidelity and operating-profile quality remain
separate qualification work.

The [direct live-browser checkpoint](../evidence/gate-6/browser-live-recovery.json)
now composes the actual `BrowserTransportClient` with that topology. Three initial
lanes pass: Chrome EdDSA/ES256 and Firefox EdDSA. Each uses a distinct control-only
Device, six joined browser reopens, original scoped status/cursor recovery and
zero browser mutations; nine existing native marked-wire probes pass per scene.
The same original handoff completes once after its coordinator resumes.

Firefox ES256 failed its initial native baseline before browser authentication:
the candidate had already retired, leaving one private-reverse marked packet.
A consistent private failure snapshot was retained. An unchanged-source trace
diagnostic later passed; it does not establish a fix. The cause remains unproven
and is carried into the planned G6-096/G6-098 native failure/quality matrices.

The [snapshot-refusal boundary proof](../evidence/gate-6/snapshot-refusal-boundary.json)
reproduces a separate native custody defect: a multi-query policy refusal could
return before detecting a concurrent SQL/clock change. Changed failed reads now
report temporary unavailability and grant no IO; unchanged authorization refusals
still retire the source. One controlled before-fail/after-pass case and all 22
snapshot/private-presence tests pass. The initial one-frame native event is not
assigned this cause without its missing retirement-reason observation.

The [G6-052 completion audit](../evidence/gate-6/transport-reconnect-completion.json)
now records all four current browser/signature lanes passing on build143 after
the separately proven refusal-boundary fix. All 36 native marked-wire windows
pass unchanged thresholds (minimum 76 partner packets, zero outside audience);
24 observed browser and 24 MCP reopens preserve original status and complete
one handoff effect per scene. All owned cleanup joins pass. This completes the
local implementation contract while preserving the failed initial baseline and
its unresolved cause. Linux/remote deployment, frozen media quality profiles,
independent Gate 6 acceptance and publication remain separately unqualified.

## Saved enrollment profiles

Use the vault's profile resolver to select already approved original enrollments
without exposing bearers in application resolver code:

```ts
resolveEnrollment: vault.resolveProfiles({
    webtransport: { name: "saved-wt-enrollment", contact: trustedWtContact },
    "wss-webrtc": { name: "saved-wss-enrollment", contact: trustedWssContact },
})
```

Configure the corresponding `endpoints` from those trusted contacts. Each saved
profile must belong to the same Identity/Device and its own selected transport.
The resolver captures names, contacts, server signing trust and certificate pins,
loads the original state, checks the profile, and claims the original completion
ID with the round's cancellation signal. It never approves another enrollment or
generates replacement IDs. Current revocation or uncertain claim observation
stops selection. This also permits the first completion of an already approved
saved profile; its original intent must have been persisted before approval.

The resolver's per-contact behavior replaces the earlier one-capture-per-round
behavior, which could reuse a WT-only credential on WSS. Existing ordinary
credentials usable on both transports may be returned for both explicit contacts;
an enrolled credential cannot cross its immutable transport binding.

[Profile fallback evidence](../evidence/gate-6/browser-profile-fallback.json)
records the actual owned-Chrome positive and refusal cases. This is a G6-061
transport prerequisite. Unified typed handles, invitation UI, device selection,
capture/render and keyboard-accessible UI qualification remain G6-061 work.

The next [typed browser control checkpoint](browser-sdk.md) adds scoped
Conversation, Participant, Session and Operation handles, current-authorized
incoming invitation handles, and original-operation recovery. It runs the same
control workflow over WT and bounded WSS fallback. Media composition and
accessible UI remain in progress.
