# Parley reference scenario

G6-071 defines the reusable example in [scenario.json](scenario.json). It has
seventeen stages, explicit audiences, business gates, observable outcomes and
public SDK target-contract mappings. The owner, assistant, booker and organizer
are required; the observer is optional. Synthetic travel facts and approval
policy belong to this example.

The scenario follows the [October 13 Parley implementation status](https://github.com/eisenzopf/parley/blob/b2984928916a3b6ccca5c501457e509cd7a50df7/docs/CONFERENCE_IMPLEMENTATION_STATUS.md)
and its [task-content profile](https://github.com/eisenzopf/parley/blob/b2984928916a3b6ccca5c501457e509cd7a50df7/docs/UCTP_CONFERENCE_PROFILE.md#L210).
The prepared upstream checkout is older; no upstream files were edited. The
example uses canonical Connect operations instead of importing the older
application-specific wire dialect.

The owner opens a browser conversation with the assistant. After the closing
speech and authoritative Session end, the assistant gathers an itinerary from
the booker. An attributed affirmative reply to the organizer's call-permission
question permits the organizer invitation. Private consultation holds the same
organizer leg silent. Owner consent and an assistant action are both required
before takeover. A proposal requires an attributed owner decision for that exact
proposal before four separate follow-ups to the selected people.

The later Parley demo moves to the phone on answer/readiness. This Gate 6 example
selects the approved configured DTMF `1` profile, so wrong-digit and no-confirmation
failures can be rehearsed. Answer alone cannot replace confirmation. The policy
and stage graph are identical in fixture and live modes; only model, telephone,
SMS and contact bindings change. Binding a live definition does not activate a
provider. Named live rehearsals require their G6-093 configuration.

Run the isolated definition and transport fixture checks from the repository root:

```sh
node --test examples/parley-on-connect/tests/*.test.mjs
```

The checks validate every stage against `contracts/sdk-gate-6-api.json` and the
canonical envelope schema. Historical context uses the public validated raw
`message.history` interface. The scripted assistant always labels its output
`SIMULATED`; its output cannot become an owner decision.

`rehearsalPath(bound, { consultation, observer })` supplies the common ordered
stage definition for both fixture and live bindings. Select `takeover` or
`decline-return` explicitly; the optional observer changes only the selected
audiences. The function preserves every business gate, observation requirement
and A6 mapping. It refuses cycles and unresolved branches. Its call lists describe
available public SDK actions, including alternative status/cancel actions; they
are not an instruction to invoke every listed action. Selecting a path satisfies
no consent, readiness, delivery or approval gate and executes no communication.

`fixtures/index.mjs` composes the scripted model, three UDP SIP/RTP participants
and a Telnyx-style SMS HTTP fixture. Supply an absolute isolated state directory
under the repository's ignored `.local/`. The fixtures bind only `127.0.0.1` and
use reserved synthetic `+12025550100` through `+12025550104` addresses. Each
speaker has a distinct G.711 mu-law tone; tests independently decode the samples.
The owner phone waits for three actual prompt frames before returning its
configured RFC 4733 digit. Wrong-digit and no-confirmation profiles are available.

SIP is a deliberately small PCMU/UDP fixture profile: INVITE, ACK, CANCEL, BYE
and OPTIONS, eight retained dialogs, explicit refusal of unsupported codecs and
re-INVITEs. It does not implement carrier routing, TLS, SRTP, registration or
crash recovery. Media timestamps preserve elapsed time through a pause. Exact
duplicate invitations share one leg; ended dialogs cannot restart media. SIP
and RTP sources are bound to the original loopback peer ports. `close()` fences
new work and drains owned timers and sockets.

The SMS fixture uses a public test signing seed and bearer. Its HTTP acceptance
response says `queued`, not delivered. Delivery/failure callbacks are explicit
fixture actions, signed over the timestamp and exact raw bytes. A serialized
ledger retains up to 256 accepted effects, refuses corrupt state, and survives
restart. Equal bodies are separate intended messages: the fixture does not invent
a provider idempotency key. Withheld acknowledgement tests retain the original
accepted effect without retrying. This ledger models fixture acceptance only;
Connect's journal and current authorization remain responsible for recovery.

Tests retain synthetic state under `.local/gate-6/reference-scenario/` regardless
of the caller's working directory. They passed on Node 26.7.0/macOS arm64; the
declared Node 22 minimum and Linux lane have not been qualified here.

G6-071 verifies the shared scenario/path definition and simulated peers over real
loopback HTTP and UDP. Both consultation branches and observer choices use the
same stage path under fixture and live bindings; 19 definition/fixture checks
pass locally. Installed SDK execution belongs to G6-069 and G6-072–G6-080, which
depend on this definition task. This evidence does not launch a model, contact a
provider or qualify the full browser/telephone application journey. Those later
tasks and Gate 6 acceptance remain open.
