# General Session control

Native readiness now has an installation-owned durable source in schema92.
The engine seals a historical observation under the exact current controller,
credential, flow epoch and active port custody. The host independently acquires
current source authority and retains the first observation for that Connection
incarnation before allowing its `connection.ready` hint. Storage failure delays
the hint and retries metadata persistence without another media submission or
an idle permission reader. Recapture preserves the original fact and timestamp.
The internal fact is neither an IO grant nor a Session Active/joined decision.
The Session started/participant multicast and loss/reconnect/grace coordinator
are installed. Immutable first-start and presence metadata retain original
audience epochs; live delivery rechecks current source and recipient authority.
Last-Connection loss uses an actual activated native/telephone lifetime and
permitted pending negotiation, with a frozen bounded grace deadline. Packet
silence is not evidence of loss. Independent terminal and Record-ready hints
follow their original durable order. The local verification scope is recorded
in [Session reconnect evidence](../evidence/gate-6/auxiliary-session-reconnect.json)
and [the owned invitation-to-record journey](../evidence/gate-6/auxiliary-session-record-journey.json).

The current signed native and explicit HTTP clients expose
`session_command(session_control::Request)`. They use the same canonical service,
current authorization and operation ledger. General control and the local native multi-participant coordinator are installed. See [canonical HTTP](canonical-http.md) and the generated
[HTTP reference](../../contracts/openapi-canonical-http.json).

A Session contains conversation-scoped Participant appearances at explicit
membership epochs. An invitation accepts Participant IDs or Identity IDs of
current members of that Conversation. The store resolves Identity IDs to the
current Participant and membership epoch under the same resource authority as
the mutation. This resolution does not admit a new Conversation member. Repeating
a participant through both forms is refused, and current recipient limits apply
to the resolved identities. Conversation membership alone does not grant Session presence.
The invited participant joins by accepting. A later invitation names the same
Session and its current topology revision; it does not create another Session.

The initiator is implicit in a new Session. Use `participants: []` (wire `to: []`)
to create a Session with just that Participant. Its capability intent is
`accepted`, with one present appearance and no outbound invitation. This does not
assert an Active Session or actual media readiness. The initiator can negotiate
its own Connection and Streams, then invite more Participants on the same
Session ID at the current topology revision. A fresh invitation on an existing
Session requires a target. Exact replay of the initial solo command still
returns its original receipt after later joins or termination; current named
permission and original source authority remain required. Other Conversation
members do not gain Session access from its creation.

```rust,no_run
# async fn example(mut client: connect_client::canonical::CanonicalHttpClient,
# cid: String, sid: String, invited_participants: Vec<String>) -> connect_client::Result<()> {
use connect_client::session_control::{Action, Request};
let command = Request {
    operation_id: "saved-session-invitation".into(),
    conversation_id: cid,
    session_id: sid,
    by_participant: None,
    expected_revision: None,
    action: Action::Invite {
        participants: invited_participants,
        medium: "voice".into(),
        intent: "conference".into(),
        capabilities: serde_json::json!({"audio":["opus", "g.711-mu"]}),
    },
};
let original = client.session_command(command.clone()).await?;
let replay = client.session_command(command).await?;
assert_eq!(original, replay);
# Ok(()) }
```

Retain both the Session ID and operation ID. The SDK supplies a fresh transport
envelope ID for each exchange. Exact replay returns the recorded local outcome
under current permission, recipient ceiling and original membership epoch; use a
fresh Conversation snapshot for current state. Changed intent under a saved
operation ID conflicts, including replacing an original Identity-ID target with
its Participant-ID alias. Resolution never rewrites the original requested intent.
Authorized business rejection is retained. A storage
failure rolls back the mutation, audience, appearance, journal and receipt, so
that same original command can recover after storage is restored. A new decision
after a stale revision requires a new operation ID and the current revision.

| Canonical command | Decision |
|---|---|
| `session.invite` | Create a new Session, or invite explicit Participants to an existing one. The latter requires its current topology revision. |
| `session.accept` | Accept the caller's pending invitation and declared compatible capabilities. |
| `session.reject` | Reject the caller's pending invitation; preserve other accepted/pending Participants. |
| `session.cancel` | The creator or a current manager cancels an unaccepted Session invitation. Once any invitee accepts, use `session.end`, including when other invitees are still pending. |
| `session.update` / `participant-leave` | Leave one Session appearance, fencing only its own Connections. |
| `session.update` / `role-change` | Change a Session role label without changing Conversation defaults or another Session. |
| `session.update` / `medium-upgrade` | Add already accepted media to the desired Session profile. Settled current native Connections may remain connected; new media needs separately negotiated Connections. Pending, foreign-runtime and legacy media bindings refuse this lane. |
| `session.update` / `codec-renegotiate` | Change declared codec preferences while no media is connected. New native offers use the latest preferences within current accepted capabilities; live Connection changes use the separately scoped `connection.update` coordinator. |
| `session.end` | Any current present Session Participant with `session.end`, or the creator/current explicit manager, ends the logical Session. Physical teardown remains separately observable. |

Session codec preferences rank compatible formats during a new native offer.
Only formats in the peer's original offer and the current accepted Endpoint and
Session profile can be selected. An unavailable preference falls back to the
peer's remaining usable offered formats. Preferences do not rewrite the signed
offer, narrow the advertised capability set or grant a codec. Replaying an older
Session update returns its historical receipt and cannot reset the latest
preferences or alter established bindings. Changing all live Connections through
a Session-wide codec coordinator remains unfinished; the current explicit refusal
preserves existing Session policy and media.

Every mutation of an existing topology requires its current revision. Pending
reject/cancel retains the documented optional revision assertion. `by` and `from`
are checked assertions of the authenticated Participant. The named action
permission is required. Ending the Session is available to a current present
Participant without `session.manage`; a pending invitation, departed appearance
or Conversation membership alone does not supply that right. Other management
actions require the Session creator or explicit `session.manage`/administration
authority. Role labels grant
no permissions. The current recipient ceiling covers affected Participants;
a narrow credential cannot end a wider Session.

Capabilities are a bounded JSON object (16 KiB). The current control profile uses
codec-name arrays under `audio`, `video`, `screen` and `data`: `opus`, `g.711-mu`,
`g.711-a`, `vp8`, and `reliable-ordered`, respectively. Recognized fields require
unique, bounded codec names. Unknown optional fields remain in the retained
intent and never grant authority. A peer may accept a usable subset, allowing an
audio-only participant in an audiovisual Session. Declared compatibility does not
prove Endpoint ownership, connection negotiation, ingress/egress flow or readiness.
Those checks remain required before media can be exposed.

The first Session is the Conversation's main Session. A concurrent Session has
its own presence, role and topology revision. Ending the main Session preserves
its identity and the other Session; a subsequent explicit creation can become
the new main Session. Session end does not close the Conversation. Conversation
closure remains governed by its explicit policy and outstanding work.

Snapshots expose a general Session view and separate authorized appearances and
Connections. The Session view includes its declared offer and preferences, not a
fixed initiator/counterpart pair. Current participant and recipient filters apply
before pagination. An original journal fact retains its own Session revision,
source and actor-only operation correlation; it is not rebuilt from current
state. Historical sharing or readmission cannot revive an earlier epoch's live
presence or operation authority.

The current state schema is **97**, with an explicit fresh-state boundary. Existing state is
never silently converted. Test defaults use new isolated state. Credentials and
raw proof logs remain under ignored private state.

`docs/evidence/gate-6/session-control.json` records the earlier schema 35 control
qualification. [The current runtime evidence](../evidence/gate-6/general-session-runtime.json)
adds real signed native media and failure/recovery cases. The local suite covers
four participant kinds and six Connection slots, membership removal and Session
leave preserving other audio, denied camera/screen upgrades, isolated stream and
Connection closure, roles, accepted upgrades, future-source reconciliation, data
and codec changes. Browser, Linux, remote/provider and frozen performance lanes
remain separately assigned plan work; local receipts never assert their readiness.

The canonical listener now installs one Core/QUIC adapter and retains the
accepted native transport in its handler. Signed Session controls use the
actual dispatcher-owned physical peer. A disposable Core Session uses the exact
durable CID/SID; a present participant attaches a bounded peer view under the
original Store command fence. The membership view preserves the durable access
epoch and uses the current Conversation revision as its governing version.
Pending invitees are not admitted Core media participants. The runtime container
is mixed-media; each Connection still needs current, explicit capability and
media authorization before negotiation or readiness.

HTTP decisions with a current present actor also project this empty Core Session
and its checked participant coordinator under the original command fence. The
container owns no physical peer or admitted media Participant. HTTP acceptance
alone creates no Connection or media reader. A later signed command can attach
its actual peer to the same container. This supplies shared runtime identity for
future initial telephone admission; it does not implement or qualify that lane.

The listener owns one bounded datagram reader per negotiated QUIC transport and
one native data queue wrapper. Receiving a datagram does not authorize a route.
Disconnect releases that physical incarnation's peer views; reconnect can attach
a new view after current authorization without recreating the logical Session.
Signed and HTTP Session end drive Core retirement while retaining their existing
Store command guard. Replaying an older invitation after end returns its original
receipt and does not recreate a Core Session.

The native runtime composes Connection preparation, current media policy, signed
parameter confirmation and managed participant admission. Explicit native audio
and video subscriptions and an additive live video upgrade are verified in the
local cases below. Stream lifecycle, membership reconciliation and the qualified media profiles are described
in the linked native guides; other platform/profile qualification remains separately scoped.

Native Connection preparation now has a private durable phase. Its `nprep_…`
ledger ID is deterministic from the original wire operation ID and remains in
that actor's operation namespace. The receipt identifies `connection.prepare`;
the wire operation ID stays in the immutable offer record for negotiation.
Completing this phase does not complete the public `connection.offer` exchange.
The public QUIC offer handler now owns the actual answer, signed confirmation and
managed admission path. HTTP has no physical media peer and cannot negotiate an
offer.

Fresh schema 36 stores one original offer and one server-issued Core Connection
ID per wire Connection ID. A preparation requires a current original Session
audience/epoch, present appearance, accepted media/codec intersection and a
verified endpoint owned by that actor. The native backend additionally binds the
exact signed intent to its admitted Device/key and held current Store readers. Creation,
private receipt and immutable journal facts commit together. Recovery reuses the
original logical preparation and Core mapping; replay reads its original result
and requires a fresh current binding check before any media work.

The backend fixtures exercise six durable preparing slots for four participant
kinds, storage rollback/recovery, endpoint/spoof/camera/stale refusals, expiry
behind SQLite, isolated leave fencing and capacity retained through retirement.
These Store fixtures do not allocate physical resources. Separate production
native tests now exercise six distinct Connections on four signed physical peers,
covering all four identity kinds. Those results qualify native allocation,
confirmation and admission; routed multiparty audio remains a separate check.


The wire offer now has its own asynchronous class in the shared operation ledger.
It uses the caller's wire operation ID and links the private preparation, original
member epoch, Device/key, source expiry and recipient ceiling. Acceptance returns
`accepted`, without a result. A protected single attempt and `submitting` native
media effect are durable before any endpoint allocation. An elapsed or abandoned
attempt becomes `unknown`; replay cannot issue another attempt. Source expiry
before an attempt fails the operation and retires the unused slot. Cancellation
requires its named capability, and an attempted exchange remains unresolved until
attributed local cleanup evidence is available.

`Store::prepare_native_offer_with_peer` composes local preparation and wire status
under the admitted native source. It checks saved wire/private receipts before
selecting an endpoint, so replay does not repeat failed preparation. A retained
local business refusal creates a separate terminal `failed` wire record with the
original operation ID, intent and asserted revision. Its Connection binding is
absent, even if another operation owns a Connection with the asserted ID. The
failure remains unchanged on replay, endpoint retirement or restart and cannot
start an effect. Current owner, original member epoch, Session audience, named
permission and recipient ceiling remain required for status. This Store control
composition performs no physical negotiation.

The native planner can also refuse a successfully prepared, accepted offer before
an attempt is issued. That checked backend path records a fixed planner failure
and retires the untouched slot atomically. It is unavailable after the attempt
marker, even if no physical result has been observed. Native profile planning
must precede attempt issuance; actual negotiation remains separate.

A never-started cancellation or expiry now commits a typed `connection_retired`
fact for each original recipient together with the slot retirement and operation
outcome. Journal storage failure rolls back that entire change. Receipt capacity
reserves one first cancellation decision independently of journal capacity;
journal reservations preserve the original recipient fanout and terminal fact
through uncertain recovery and attempted cancellation. The first uncertainty and
cancellation facts consume separate reserved room. An attempted effect still
requires actual runtime cleanup evidence; missing observation cannot retire it.

These pre-effect Store paths cannot mark media ready. The installed runtime
completes a wire operation only after actual native resource confirmation and
managed admission.
Endpoint selection requires one already verified QUIC endpoint for the actual
admitted Device, preserving a saved selection on exact replay. Ambiguity fails
explicitly. Selection and dormant slot insertion retain parent readers, so they
cannot provision endpoints or change permissions during the network exchange.
Current receiver checks enforce both sender and receiver recipient ceilings;
joining a Session does not widen an earlier sender's credential.

Media origins require the exact server-issued native attempt. Every fresh
packet read checks that attempt against the shared ledger. The initial bounded
control invocation retains its original source readers and checked view through
engine callbacks, where SQL or authority reacquisition would be unsafe. Source
authorization stays fenced; the attempt is checked outside engine fences before
negotiation, again before admission, and by the final durable commit. Cancellation
can finish parameter exchange but cannot admit or mark the Connection ready. Cancellation, uncertainty, a lost worker or an elapsed negotiation
deadline fences the origin. A fresh signed proof cannot extend the accepted
source's original expiry. The origin retains metadata, never an idle authority
grant. Codec planning first filters the server profile by the current Session
offer, accepted codecs and verified endpoint capabilities, then applies the
peer's preference order to the permitted choices.

The Rust canonical client can send a fresh signed `connection.answer` bound to
all four original offer scopes and exchange bounded native RTP datagrams. An
atomic datagram timeout or cancellation preserves the signaling peer. These
helpers do not confer media admission. The server now derives readiness from the
actual native activation, controller revision and managed Core Connection under
the original source fence.

Fresh schema 41 retains a separate native Connection lifetime after a successful
wire operation completes. Its cleanup reservation survives completion; the
original pending reservation funds readiness, completion and eventual retirement
facts, including uncertainty and cancellation. A signed answer is only a parameter
observation. `connection_ready` commits with the confirmed effect and completed
wire record after the engine observation is checked again. The original operation
remains completed when that Connection later retires.

A bounded owned worker monitors each actual native route. Peer EOF, current source
retirement, Session leave/end or Conversation closure ends only the affected Core
route. Adapter absence must be observed before retirement and quota release; a
missing reply cannot claim absence. No idle invocation grant is retained. Native
callback approval binds the original frame, scopes, participant, credential,
Device/key, codec/direction selection and exact signed confirmation parameters.

Before successor intake, exclusive kernel installation ownership and actual local
native adapter absence can settle old native attempts. A pending attempt becomes
failed after confirmed cleanup; a completed creation preserves its historical
result and effect identity. Neither case resubmits an offer on replay. This
recovery path does not claim anything about provider resources.

Production tests cover signed native exchanges and EOF retirement, six admitted
slots across human/AI/external/system peers, denied camera upgrade, isolated leave,
explicit Session end before Conversation close and abrupt completed/interrupted
restart. The previously ignored routed-audio case now passes with an explicit
participant audio subscription, both before and after another participant leaves.
The current cases qualify PCMU routing and VP8 resource selection with exact
frame assembly and an independent decode. They do not qualify the frozen
60-second AV performance targets, every codec or remote/browser/Linux profiles.

Signed QUIC `stream.subscribe` and `stream.unsubscribe` now use an owned runtime
worker and the same shared operation ledger as other mutations. Each requires its
own named permission, current Device/key, original source credential, participant
epoch, accepted capabilities and ready generation. The actual adapter route must
also belong to that same physical peer. Another physical peer using the same
Device/key can read current authorized operation metadata, but cannot execute the
old route's control. HTTP may query or cancel the retained operation; it cannot
supply physical media custody.

The stored retry identity includes selector order and optional extension fields.
Rust `CanonicalClient::send_value` preserves the original optional payload fields
through envelope validation and signing. Typed `send` remains available for typed
payloads. A changed extension under an existing logical operation ID conflicts.

Schema 41 retains one open control per native Connection and marks one uncertain
effect before the engine may act. Preflight failures and stale revisions retain
failed records without executing an effect. Exact stream selectors and audio
participant selectors use a separate bounded control lease with current target
and source readers. Engine callbacks compare the exact planned change and reuse
those readers without Store reentry. Every packet obtains fresh current source
and recipient checks, including the receiver's separate `stream.subscribe`
permission; Connection creation alone cannot authorize selected reception.

Only the actual returned engine outcome can create a completion receipt. A known
success consumes the attempt into a receipt that can retry durable recording
without repeating the engine call. It advances the distinct stream revision only
for the still-live original generation. Completion after actual retirement records
history without reviving the slot. If recording fails, the caller receives pending
status and the owned worker drops the original command readers while retaining
only that receipt. The production storage-failure case proves source writers can
proceed, a late cancellation remains uncertain, and later recording preserves the
same root/effect IDs while qualifying the actual routed audio. Generic engine
errors and restart uncertainty never invent successful completion or resubmit.

The exact-stream production case verifies successful routing, stable replay,
conflicting extension refusal, another physical peer's no-effect refusal,
unsubscribe silence and successful resubscribe on the same physical connections.
Receipt and journal reservations protect cancellation and outcome recording at
capacity. Non-audio participant reconciliation, publication stream removal and ordinary
membership/runtime reconciliation remain open G6-038 work.

Current media permissions derive from the accepted endpoint and codec
intersection, independently of which streams were selected in the initial offer.
This permits adding a compatible video stream under the exact current control
plan. It does not create a port or media grant from a capability declaration.
The real video test rejects a stale signed confirmation, verifies audio keeps
flowing while that same negotiation waits, then accepts the correct confirmation
under the same effect and routes a 640×360 VP8 keyframe. Complete RTP marker,
sequence and codec bytes survive SDK assembly; the captured frame also decodes
with FFmpeg. An endpoint verified for audio alone cannot bypass that restriction
by advertising VP8, and its rejected attempt leaves existing audio usable.

An additive live upgrade keeps the original audio Connections and their
negotiated profiles. The caller must already have accepted compatible codecs in
the original Session offer. The metadata result changes desired media, not actual
readiness; new video then uses separately signed and negotiated Connections in
that same Session. A production case verifies audio before the update, after the
update and after actual VP8 routing on the two added Connections. Replaying the
upgrade does not create more Connections. Existing port codec renegotiation and
other upgrade profiles remain separate unfinished work.

Pure snapshots and journal projections retain the same validated operation source
readers. They do not take the metadata writers used by operation recovery. This
allows authorized snapshots while an original native exchange is awaiting its
signed confirmation; real mutations still exclude those readers.

The runtime registers its cancellation token with installation retirement, so
abrupt server drop cannot leave workers retrying against a retired owner.
Graceful shutdown stops and drains the policy worker while installation authority
is current, then retires ownership.

Fresh publication and delivery also require a durable ready/active slot, after
the same transaction commits the completed wire record and confirmed effect.
Engine preparation and source control views cannot authorize media in that gap.

Signed QUIC `connection.end` now uses this same single-attempt control custody,
with its own named permission and the original route's physical owner. The
original bounded reason and optional extension fields remain part of the exact
logical identity. An optional expected revision must match the current native
stream revision; omitting it is retained distinctly from supplying it. A new
termination requires a current settled source; a replay after retirement returns
only the current authorized historical metadata.

The owned worker calls Core once and requires both its actual success and absence
of the exact original native route before capturing an end receipt. A timeout,
restart or generic engine error stays uncertain. Recording that receipt grants
no media permission, advances no stream revision and never revives the retired
slot. The original creation worker independently settles the durable lifetime
from actual adapter absence. Ending one Connection preserves the other Session
Connections and the physical peer, so the caller can continue authorized audio
and negotiate a fresh Connection. Participant selectors reconcile later publication and source retirement through
current policy; see [native subscriptions](native-subscriptions.md).

Screen is an explicit media kind in Session acceptance and additive upgrades.
The `mixed` desired profile includes audio, data, screen and video; each kind
still needs its own usable codec intersection before allocation. Screen uses
VP8 and retains `kind: screen` and its own logical stream ID throughout offer,
confirmation, selection and routing. It does not borrow `video` permission:
the original Session offer, that participant's accepted capabilities, the
Session's desired media and the verified Endpoint must each permit screen.
Malformed or duplicate screen codec declarations are refused.

The production native screen cases qualify an audio Session upgraded to screen,
actual signed screen Connections, exact screen subscription, received VP8 frame
assembly and independent decode, while the original audio continues. Closing
one screen Connection preserves audio and the Session. A video-only Endpoint
and a participant who accepted camera video but omitted screen each refuse the
screen offer before a native effect, preserving audio. These are isolated native
routing cases with a synthetic frame. Browser capture permission, live screen
capture and the frozen AV performance profiles still require their own evidence.
Reliable data uses durable physical writer outcomes; see
[native reliable data](native-reliable-data.md).

Reliable data source admission is now available to the trusted runtime. It
requires the exact native parent credential, Device/key, participant epoch,
current Session/Endpoint capability intersection, an observed sending data label
and the separate current `send` permission. The protected original payload keeps
message and operation IDs, label, addressed Participant order, encoding and
optional payload extensions. Text and binary bodies share a 64 KiB decoded limit;
binary uses canonical Base64. Unsupported delivery modes, attached files and
reply-message references are refused in this native profile.

`data: ["reliable-ordered"]` declares the Session delivery class. Native
`text`, `json` and `binary` formats still require their own explicit offered
protocol intersection; the class grants no arbitrary protocol. Current data
planning returns audience Participants at their current membership epochs and
requires accepted Session presence and original Session visibility. Queue
admission must persist that audience before publication, so later joins cannot
expand an already admitted message. Source metadata is not an engine grant or
a delivery receipt.

Source projection itself creates no operation, journal fact or engine effect.
Fresh schema 42 additionally installs durable native-data admission in the
trusted runtime API. The producer must verify its actual physical source and
selected active format before writing a new visible message fact. It atomically retains the complete original intent, writes
one Message through the shared message journal and freezes recipient identity
and membership-epoch snapshots. It shares message and logical-operation ID
namespaces with ordinary sends. The native path skips ambient generic provider
fanout. Admission is local durable acceptance; it does not prove queue entry or
delivery. Later cancellation of native publication preserves that message fact.

A single owned attempt is marked before Core may publish. Only the actual
returned queue-admission success may consume that attempt into its attributed
recording receipt. The result explicitly says `core_queue_admission: true`; it
does not advance a stream revision, claim a physical write or assert remote
consumption. Captured results can recover storage failure and late cancellation
under the same IDs. A successor runtime preserves uncertainty without repeating
a possible publication. Replays use the original audience, so a later join cannot
expand an admitted `all` message. Frozen role/presence metadata is a historical
snapshot; actual delivery still requires fresh authorization.

Native data now negotiates `text`, `json` or `binary` ports through the signed
native connection path. Publication uses `message.send` with all of `cid`, `sid`,
`connid` and a stable `operation_id`, plus required `msg_id`, `label`, explicit
`to`, `content_type` and `body`. Native scoped sends are unavailable over HTTP;
HTTP operation status and cancellation remain available. A new message must pass
the actual physical-source and active-format checks before durable admission.
Changing physical peers cannot fail or take over an existing pending root.

The initial publication reuses its exact signed invocation's bounded readers.
Queued delivery obtains fresh source, target and message readers, checks current
send/history/subscription permissions and the original audience epochs, and
reserves writer capacity before possible IO. Recipient omission from the original
snapshot cannot be repaired by later presence or history grants. A statically
excluded route can acknowledge its actual refusal for resource cleanup without
creating a recipient effect or exposing a route fact.

Schema43 records immutable `native_data_write` events for addressed targets.
`local_written` means a local physical write, while `not_started` and
`submission_unknown` distinguish refusal and uncertainty. These facts neither
complete the source queue operation nor assert remote delivery or consumption.
A successor converts pending predecessor reservations to explicit runtime
recovery uncertainty and never resubmits the original message. Storage recovery
retries the captured outcome under the same identities. Actual signed local QUIC
checks cover routed IDs/payloads, replay, physical-source refusal and writer
storage recovery alongside existing audio. Other data/media profiles and platforms still require their assigned qualification.


[Native codec controls](native-codecs.md) document actual signed renegotiation,
captured results, and terminal Session cleanup. The current production canonical
listener uses general Participant appearances and durable Session IDs. Named
provider/task adapters and retired generic SDK consumers are not evidence for this
canonical lane; their remaining integration and migration belong to G6-044,
G6-059 and G6-065. No compatibility dispatch is restored for them.


## Pending invitation sources

A successful invitation now captures its original inviter and each selected
Participant/epoch in the same mutation savepoint. Repeating the operation keeps
that source; a later invitation to the same Participant has a different source.
Protected runtime readers recheck the exact pending slot and current permissions.
This is the foundation for server-owned invitation exchanges, not a claim that
forwarded `session.invite` and correlated answers are installed. See
[invitation sources](session-invitation-sources.md).

## Receiving a native invitation

`receive_session_invitation` receives a signed `session.invite` on an authenticated
idle native peer without
a media Connection or a Conversation subscription. `answer_session_invitation`
sends an exact correlated `SessionInvitationAnswer::Accept { capabilities }` or
`Reject { reason_code, reason }` on that physical peer. This sends an observation;
it does not allocate a caller operation or return a durable decision. Observe the
authorized Conversation snapshot/journal for the committed Session result. A
late, wrong-scope or old-credential answer cannot enter command execution.

Canonical invitation media are `voice`, `video`, `voice+video`, `screen-share`,
`text-chat` and `mixed`. The former pre-alpha `voice-video` and `data` invitation
spellings are not aliases. Codec arrays still use `audio`, `video`, `screen` and
`data`. Acceptance alone establishes presence; actual Connection readiness is
separate. Committed decisions can relay to the exact original native caller;
logical invitation replay recovers the stored decision with a new request
correlation. Bounded recipient cancellation observations use the exact forwarded
invitation ID. The native caller can assert its own original invitation request
ID through `cancel_session_invitation`; replay the logical invitation after
reconnect to register a new reference. URI gateways and task-origin policy remain
explicit open work.
