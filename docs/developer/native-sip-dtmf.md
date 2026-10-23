# Native DTMF to a connected SIP call

The G6-046 runtime can send an actual signed native `dtmf.send` to an existing,
connected initial or committed replacement SIP call in the same Core Session. It uses the public native
SDK command and the existing `NativeDtmfSubmission` operation result. Completion
reports local submission, not remote tone playback.

After the Store records the original call as connected, its actual host leg
receives one registration ID. Native ingress selects current registrations and
atomically freezes the intended native and SIP recipients in the existing
schema100 batch. The queued batch retains metadata, not permission readers.
There is no redial, second logical operation or synthetic native SIP recipient.

Each SIP schedule keeps one stable attempt ID. Before physical IO it reacquires
the independent current `dtmf.send` and `dtmf.receive` permissions, exact source
and answered call, original Endpoint/Device and route credentials. It reserves
the schedule durably before possible IO. Reservation contention retries metadata
with that same ID after releasing the readers. The actual output uses Core's
original route and the requested INFO or negotiated RFC4733 method once.

`native_sip_dtmf_audience_fence` binds the sealed original native disclosure to
the actual registered SIP controller and principal. During each IO poll it
checks original and current scene groups and holds the SIP target's current
credential interval alongside the existing native source and physical route
fences. It does not reenter the native source controller. Pending releases the
synchronous scene and target guards. Rebinding, retirement and a return from a
private scene cannot lend a new audience to old work.

The schedule's fresh signal read lasts at most 200 ms and never constructs a
media permission. The initial call monitor also permits an existing call to
remain while a current named signal direction is authorized, even if its media
permission is withdrawn. RTP remains subject to its own packet guards. The
physical signal-only rehearsal withdraws all telephone credential operations
except `dtmf.receive` under the actual Store writer fence, drains previously
admitted packets, then verifies marked RTP stops while the same connected call
still accepts both INFO and RFC4733. This controlled credential edit does not
qualify a public credential-edit API.

After actual writer leases stop, the runtime records local submission,
not-started or uncertainty and settles the shared mixed batch. Recording retries
cannot repeat IO. Replay recovers the original journal result. Restart retains
the Store's reserved-unknown versus unreserved-not-started distinction.

The production fixture uses real local UDP SIP on one original dialog and runs
EdDSA and ES256. INFO checks the actual Contact Request-URI, dialog, digits,
duration and reserved journal row before acknowledging it. RFC4733 checks the
negotiated payload101, 8 kHz clock, exact 100 ms event duration, three digits and
redundant terminal packets. Both methods verify replay creates no extra tones.
The independent receive-permission refusal runs through the same signed SDK.
The fixture retains its existing marked duplex audio thresholds and cleanup.

A further production case returns SIP403 after the first actual INFO. Both
signing algorithms stop the original suffix, retain the reserved attempt as
`outcome_unknown`, and recover the same uncertain journal result on replay
without another packet. After the peer accepts INFO again, a fresh logical
operation submits all three digits with a different schedule ID on that same
call. This verifies local failure/recovery without claiming remote playback or
retrying the uncertain operation.

Cancellation and source revocation are also exercised after the peer receives
the first actual INFO of a three-digit, 1000 ms sequence. The remaining suffix
stops, one reserved attempt remains uncertain, and the call stays connected.
An authorized replay of the cancelled operation returns that same uncertainty;
the revoked source cannot replay, and a fresh credential of the same identity
can read the original result. The quiet observation crosses the original tone
duration and gap. Both signing algorithms execute these cases.

The maximum RFC4733 fixture requests five 5944 ms digits with four 70 ms gaps:
exactly 30 seconds of validated tone/gap time. It checks all five exact 47552
sample terminal durations at 8 kHz, redundant end packets, one original call,
one completed schedule and no replay. Its local elapsed bound is 35 seconds.

DTMF admission and Core/INFO execution have a separate 40-second budget,
narrowed by the original validated source, credential and caller deadlines.
Replay cannot refresh it. Other native stream controls retain their 30-second
budget. The packet's fresh signal grant remains bounded to 200 ms.
RFC4733 packet cadence follows the monotonic tone clock after first submission,
skipping obsolete continuation ticks when permission work is delayed. Fresh
authorization still surrounds each emitted packet, including every final one.
The controlled UDP delay case checks actual tone wall time and one grant per
actual packet, alongside exact final sample duration and terminal redundancy.

The native writer keeps metadata admission within its original one-second job
deadline. The 200 ms physical write budget begins after the host returns a fresh
lease; the lease's own current-authority deadline can shorten it further.
Metadata contention therefore waits without retaining permission readers or a
writer, and cannot renew the original source or job deadline. Controlled
300 ms metadata delays check successful submission, an actual stalled sink's
short IO cutoff, and original deadline/cancellation refusal before any IO.
SIP INFO similarly keeps metadata admission within the original sequence
deadline and starts its 200 ms physical budget after fresh host authorization.
An actual two-call Core/UDP test holds the independent permission writer for
300 ms, then checks one original INFO request. Shorter original expiry and
cancellation still refuse without a late packet after the writer releases.

Frozen native and SIP recipients now run concurrently, with at most Core's
64 active schedules per batch. Additional recipients queue only original
metadata and keep the original expiry; waiting cannot renew a source grant.
The parent awaits every started worker's physical cleanup and recorded outcome,
even when another recipient fails. Settlement never abandons healthy work or
converts a partial result into completion. Each recipient retains the same
per-write authority, stable attempt ID and no-IO-replay behavior.

A two-dialog production fixture admits separate verified telephone identities,
Devices and Endpoints through the public Session invite. Each actual peer gets
two 6000 ms RFC4733 events (12.07 seconds including the gap). Both full schedules
must fit 18 seconds, their observed first packets must be within one second,
and each event must end at exactly 48000 samples with three redundant ends.
A separate INFO case refuses one recipient's first digit while the other
finishes both digits. The operation stays uncertain with one submitted and one
uncertain recipient, records both outcomes and replays without another packet.
Both algorithms check separate schedule IDs, one call per peer and cleanup of
both original dialogs. These are two-recipient checks, not the full 50-member
or every-substrate qualification.

These checks qualify a bounded local macOS arm64 component. Other maximum-duration profiles,
larger fanout, target/route revocation, restart/socket uncertainty and remaining
private combinations need further implementation or production checks. They do not
close the cross-substrate DTMF clauses, G6-046, independent G6-094 conformance or
Linux/browser/remote/provider profiles.

Committed SIP replacements use the same native command. The runtime discovers
only the actual current replacement controller and freezes its original
registration. Schema102 binds that recipient to its telephone lifetime,
generation, SIP call ID and configured route credential. It creates no initial
call record or native lifetime for the SIP recipient. Current `dtmf.receive`
on that route and `dtmf.send` on the native source remain independent of RTP
and subscription permission; the established route also requires `sip_dial`.

Every INFO or RFC4733 write checks the exact replacement binding and current
original audience. Queued work holds metadata, and a new current-authority
reader covers each physical write. Reservation failure retries metadata for
only the original schedule. A captured local outcome can settle after rights
loss; restart preserves reserved uncertainty and never replays the schedule.

Schema103 also records actual received input from a committed SIP call. The
installed monitor subscribes with that call's original controller and route
principal, then freezes INFO or RFC4733 input and original native registrations
without holding permission readers in a queue. Initial and established calls
share typed signal admission and delivery; each retains its actual lifetime
kind, generation and call identity. Received operations belong to the logical
participant. An independent route credential supplies current signal permission
and does not become the operation owner or reuse a retired native credential.

A fresh source-owner HTTP credential can inspect the original result. Repeated
physical input cannot recapture recipients or refresh the original deadline;
replay rechecks current source permission and original recipient ceilings.
Every actual native write uses a fresh complete signal interval and the sealed
original/current Core audience. Media and signal permission stay independent,
and a retained signal direction can keep the call alive while media is denied.
Private combinations, wider source profiles and later hosting/transport
qualification remain separately recorded work.

An observed native `auth.refresh` preserves the Connection, Stream and logical
participant. Fresh DTMF source commands and newly captured native recipients
use the applied credential epoch. The admission view resolves that current
epoch while retaining the original offer record.

Queued SIP input keeps its original recipient registration and credential
epoch. Refresh can finish while reservation metadata waits without permission
readers. If no frame was reserved, that old recipient stops as `not_started`;
new input can use the new registration. Existing reserved/uncertain outcomes
retain their original rules. Replaying an old operation cannot retarget it,
refresh its deadline or submit its signal again.

A native private consultation includes every current initial and committed moved
SIP outsider in its exact audience scene. Each keeps its actual call lifetime,
route owner, generation and current media permissions. The outside read permits
unchanged public programmes and private exclusion; it cannot become a private
slot or a native signing projection. Captured audience outcomes update only the
original live call's next Stream revision. Return uses fresh current readers
and carries the actual original controller's audience receipt. Old DTMF input
keeps its original sealed audience after return.
