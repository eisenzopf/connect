# Native private custody before signal application

The native worker's idle liveness probe uses Presence validation. This preserves
the current original native credential, signing key, Device, Endpoint, member
epoch, Session access, private context and runtime ownership checks, and returns
no packet or signal reader. Quarantined and prepared native consultants can
therefore remain owned while their private audience is being prepared.

The old probe used Signal validation. Signal access correctly requires captured
audience application, so the worker treated a newly ready, still muted consultant
as unauthorized and ended it. Read-only socket diagnostics captured readiness at
generation 1, then retirement at generation 2 about 100 ms later. Consultation
preflight subsequently refused that retired consultant's native projection, or
found no ready candidate and remained Accepted.

The regression test reproduces the pre-application liveness refusal before the
fix. It then checks custody before and after consultation preparation while
signal facts remain unavailable, current original authority withdrawal still
refuses custody, the source remains ready at generation 1 and no idle reader
blocks the next writer. Media and signal IO retain their existing purpose-bound
authorization. Presence does not supply named DTMF rights or a private programme.

Actual initial/established SIP-primary socket profiles and private SIP candidate
quarantine/cleanup regressions are qualified separately against the combined
source. Passing controlled metadata tests does not close G6-046 or its DTMF
requirements. Public commands remain in the [server HTTP reference](canonical-http.md).
