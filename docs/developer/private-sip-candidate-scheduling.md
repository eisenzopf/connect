# Private SIP scheduling and worker isolation

Both generic transition execution and compensation now exclude any registered
consultation candidate, whether native or SIP. A reserved SIP candidate is
protected before an initial-call child, allocation, signaling or audience attempt
exists. Generic worker recovery cannot take over that root or create a parallel
effect. The protected consultation coordinator and original call custody remain
responsible for it.

The ready-leg scheduling hint also recognizes the original SIP candidate. It
requires the registered canonical ID and runtime, generation one, ready/active
connection state, completed original consultation child, confirmed initial call
effect, own private consent and accepted/connected/non-retired private observation.
Current readiness must name the same private root, Core connection and SIP call.
A public native sibling, reservation, answer consent or fabricated ready flag
without the original observation cannot supply the hint. Native candidates keep
their original registered, current native-lifetime checks.

The caller still needs current named consultation permission and visibility of
the original operation. A positive result is metadata only: it owns no idle
reader, starts no call, repeats no mutation and authorizes no IO. The coordinator
must separately acquire fresh original authority and validate its actual owned
call and private context. Foreign runtime, changed generation and captured
retirement remove the hint.

This closes the scheduling/worker-role gap before runtime SIP-candidate selection
is enabled. Complete private packet/control/signal/lifecycle intervals and actual
private audio/INFO/RFC4733/return remain required for G6-046. No clause or task
closes from these checks. The existing consultation and operation methods in the
[server HTTP reference](canonical-http.md) remain the client interface.
