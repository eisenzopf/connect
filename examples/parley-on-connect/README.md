# Parley reference scenario

The G6-072 application shell is in progress under app/. Its private intent
journal, public Node SDK gateway, local HTTP boundary and responsive UI are
implemented. Run npm run test:app here for the application component checks.
Those checks use simulated SDK-shaped clients and do not qualify installed
SDK execution.

The local browser component uses the installed browser SDK with a simulated
application backend. Its 15 checks cover keyboard participant selection and
focus, conversation/text/invitations, actual microphone/camera denial, 320px
layout and original-action offline/reload recovery. See
[the keyboard component evidence](../../docs/evidence/gate-6/reference-app-keyboard-component.json).

The independently verified installed app at a2e9b511 passes all 15 control checks
on Node 22/Ubuntu x86_64 and macOS arm64, including native keyboard participant
selection, cold scoped setup, original-ID recovery, restart and owned shutdown.
See [the installed keyboard evidence](../../docs/evidence/gate-6/reference-app-installed-keyboard.json).
That result predates the media Endpoint setup below.

The current setup also provisions one application-owned native media Endpoint
per installed Device through the public administrative SDK. It reads only public
CLI client configuration, checks signed native access and media capability, and
saves original provisioning and verification commands before submission.
Verification binds the Device, Identity, local contact, configuration and runtime.
It represents an attributed local setup decision. The independently audited
installed application at abeea973 passes all 24 checks on Node 22.23.4/macOS arm64
and Ubuntu x86_64, using Chrome 156.0.8078.4. Both browsers capture distinct
synthetic microphones, send Opus and source clocks through genuine installed
native handles, and play the other source's identified tone. Explicit stop joins
capture, rendering and packet work; original Connection end, application restart
and every owned browser/server/helper join also pass.
See [the installed audio evidence](../../docs/evidence/gate-6/reference-app-installed-browser-audio.json).
At source91bdfc90 both installed platforms also identify both remote camera
markers at decoded/painted VP8 output, with video source clocks. The full runs
still fail later control/recovery steps; this is component evidence only. See
[the installed AV component evidence](../../docs/evidence/gate-6/reference-app-installed-browser-av.json).
Full failure/restart freshness qualification remains open.

The app keeps its private setup manifest, administrator configuration and
credentials on the backend. Application recovery state remains separate from
Connect's journal and database. Only installed public SDK packages perform
Connect operations.

With Node 22.16 or later, the ordinary `@rudeless/connect` 0.1.0 package installed
here, and the matching candidate's `connect` executable on PATH, run:

```sh
npm run app
```

This creates a private `.local/parley` installation, provisions the four synthetic
participants, installs separate credentials limited to their three peers, and
prints the loopback UI URL. It starts neither a model nor a communications
provider. An explicit installed executable or application state location can be
selected with `npm run app -- --connect /installed/bin/connect --state .local/demo`.
No participant IDs, tokens or database changes are entered manually.

Setup profile 3 retains the original media Endpoint decisions and adds only the
literal connection.end, stream.subscribe and stream.unsubscribe participant
permissions needed for the new controls. Earlier profile-1 and profile-2
directories are refused unchanged. Use a fresh explicit state directory such as
`--state .local/parley-media`; existing grants and saved operations are not
expanded or rewritten to adopt the new workflow.

The UI opens audio or audio/video Connections in accepted Sessions and exposes
mute/unmute, hold/resume and original Connection end. Original Connection/Stream
IDs are saved before SDK submission. “Listen to” selects a present peer's audio;
“Watch” selects their video through the public participant/kind selector.
“Stop watching” removes only the negotiated incoming video binding, preserving
the local camera publication. Consented Sessions remain live as they progress
from accepted through ready and active; a view refresh retains their original
capture binding. Ending, closed, absent or changed scopes still stop media.
These video additions are component-tested;
installed VP8 capture/render verification is still pending.

Joining negotiates native resources. Explicit “Start microphone” or “Start
microphone and camera” begins browser permission, capture, encoding and playback.
Stop media releases local tracks, WebAudio, codecs, queued frames and requests.
Capture also stops when the page is hidden/offline or its actor, Conversation,
Connection or negotiated binding changes. There is no automatic capture restart.
The browser holds no backend credentials. Per-packet native checks and a separate
owned read-only signed control supervisor retain current authorization; retiring
that supervisor cannot cancel the client owning original actions/native media.
The AV rehearsal uses separate owned synthetic red/green cameras and requires
the other marker at decoded, painted output, with video source-clock reports;
those checks do not use a local preview as evidence of reception.

Source RTP timestamps retain each device's original clock. The application
calibrates that clock to the page's monotonic time at first accepted capture and
pairs its extrapolated current RTP point with current NTP time in sender reports.
This avoids treating a device clock's arbitrary origin as page elapsed time.

The installed rehearsal additionally checks actual tracks after explicit stop,
a controlled late device result after cancellation, real microphone permission
revocation, offline retirement and participant switching. The delayed result
uses a real owned synthetic browser stream; it models promise delivery rather
than claiming an OS permission dialog was exercised. These new lifecycle checks
require installed verification with actual ended-event/stop-cause checks. The
old fake permission-UI bypass is removed; explicit browser permission grants
remain limited to owned synthetic devices.
The rehearsal explicitly launches the installed full Chromium browser in headless
mode. It clears prior permission overrides before applying a camera-only grant
to revoke the microphone; adding another grant alone does not revoke one.

After going offline, capture and mutations stay stopped and passive polling
stays suspended until an explicit Reconnect refreshes current access. Original
Connection end happens before replacing the owning SDK client, and recovery
observes the same saved operation ID. Identical reads already in flight share
work only within the same actor, selector and SDK owner. No snapshot or
authorization result remains cached after that work finishes.

Ctrl-C joins the app, its SDK helpers and its owned server. Repeating the same
command preserves the original ports, identities, issuance IDs, absolute expiry
and application state. Interrupted child installations use `connect issue-resume`;
changed assets, a different runtime, expiry or an unsealed initial bootstrap
refuse and retain the original state for inspection. This is the local synthetic
profile; video and full lifecycle/failure qualification remain unfinished.

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
