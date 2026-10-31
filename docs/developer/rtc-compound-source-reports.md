# RTC compound source reports

G6-054's browser quality failures still require a cause and a verified fix. The
source audit identifies a separate wire-format question: the sender-report
interceptor emits a bare SR, and the actual RTC outgoing handler forwards its
packet vector unchanged. Local SDP does advertise a stream CNAME, but that alone
does not prove an RTCP source description was sent.

[RFC3550 section6.1](https://www.rfc-editor.org/rfc/rfc3550.html#section-6.1)
requires CNAME descriptions in ordinary compound reports.
[RFC8834 sections4.6 and12.2.3](https://www.rfc-editor.org/rfc/rfc8834.html#section-12.2.3)
distinguish reduced feedback and describe a shared RTCP CNAME plus correlated
sender-report clocks for synchronization. The receiver decides whether to use
those mappings. This does not establish that missing SDES causes the observed
Connect AV tail.

The counterexample patch changes tests only. Three functions execute the actual
`InterceptorHandler::poll_write` with authenticated transport state and known
local audio/video sender bindings. They require each SR to retain its original
NTP/RTP/counts and carry the exact signaled CNAME; distinct signaled groups must
stay distinct. An existing compound, unknown sender and reduced PLI must not gain
a new or duplicated description. The hop fixture passes packets through the
same protocol handler; no copied report formatter substitutes for production.

The `rtc-compound` Linux lane compiles the exact pinned RTC crate, verifies the
three-function inventory, executes each function separately and seals the
binary/source inputs. The full Linux fixture lane requires the same component.
At source `9b281440`, run `41208154602`, the first two assertions fail at
line116 and the existing-compound/unknown/reduced-feedback assertion passes.
The artifact verifies4,538 Git inputs,4,094 prepared inputs and four pinned
archives. This reproduces the wire gap in the actual handler; AV-tail causality
and the corrected behavior remain unqualified.

The first external preparation failed with ENOSPC before selection. Only that
fresh generated root was removed. A new internal ignored preparation reproduces
all258 recorded patches and pinned source digests; the prior frozen73 and main's
frozen71 remain intact. No local Rust build was attempted.

The correction appends source descriptions only for active, unambiguous local
sender bindings and their negotiated repair SSRCs. It uses the exact track stream
ID already used by local SDP, preserves SR clock/count fields and existing
source descriptions, and does not relabel unknown or stopped sources. Empty or
overlength names are refused. Each SDES packet has at most31 chunks. Reduced
feedback retains its original format; no media permission or source clock is
created by this metadata operation.

The original three assertions remain byte-identical. Two additional actual
handler tests cover stopped/inactive/ambiguous/invalid bindings and a32-source
batch across the five-bit SDES count boundary. The after-run also executes the
current RTC unit inventory and records any declared skips explicitly. The
correction is prepared in fresh verified internal frozen75; no local Rust build
or corrected runtime pass is claimed yet. Original strict browser/audio/video
and recovery verification remains required.

The first after-run (`94116f8d`, `41208970825`) verifies its source inputs but
cannot execute tests: Rust E0283 points to the added32-source test's expected
collection type atline205. The follow-up patch explicitly names `HashSet<_>`;
assertion semantics and the original three functions remain unchanged. The
failed compilation is retained as unrun, with no component pass. Fresh frozen76
reproduces all260 recorded patches and source digests before the next run.
