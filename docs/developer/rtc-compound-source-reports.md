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
The new assertions are currently unrun. They are not a wire/component pass,
browser quality result or G6-054 qualification.

The first external preparation failed with ENOSPC before selection. Only that
fresh generated root was removed. A new internal ignored preparation reproduces
all258 recorded patches and pinned source digests; the prior frozen73 and main's
frozen71 remain intact. No local Rust build was attempted.
