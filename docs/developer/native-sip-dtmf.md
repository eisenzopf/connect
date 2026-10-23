# Native DTMF to an initial SIP call

The G6-046 runtime can send an actual signed native `dtmf.send` to an existing,
connected initial SIP call in the same Core Session. It uses the public native
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

These checks qualify a bounded local macOS arm64 component. SIP-originated
observations, established/replacement SIP legs, maximum-duration sequences,
larger fanout, target/route revocation, restart/socket uncertainty and remaining
private combinations need further implementation or production checks. They do not
close the cross-substrate DTMF clauses, G6-046, independent G6-094 conformance or
Linux/browser/remote/provider profiles.
