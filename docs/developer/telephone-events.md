# Telephone-event primitives

Gate 6 task G6-024 remains in progress. The negotiated SIP/WebRTC component
checkpoint is [sip-dtmf-profile.json](../evidence/gate-6/sip-dtmf-profile.json).
It records 401 successful local test executions on one unchanged prepared source,
including actual SIP UDP and WebRTC SRTP. Its preserved generation is54; later
changes require current affected checks. Reproduce with
`scripts/check-gate6-sip-dtmf-profile.py`.

The protocol rules and custody boundaries are in
[native-signals-profile.md](../../contracts/native-signals-profile.md).
The RTP transport never interprets PT101 by itself. Explicit negotiated receive
profiles select the dynamic payload, clock and supported event set. A sender
uses the remote endpoint's receive capability, while a receiver checks its own
advertised set. A restricted suffix rejects the entire request before any tone.

Authorized SIP sends compose host permission with the exact current media
configuration, resource and session transaction lane for each real UDP write.
Authorized WebRTC sends acquire permission inside the owned driver submission
through encryption and socket IO. Every final repeat needs a new permission;
idle tone ticks and digit gaps hold no reader. Failure after a submitted prefix
remains uncertain for the complete original operation.

These APIs are transport primitives. General Core/native gateway routing still
needs actual source and recipient policy, requested method and stable operation
identity. A successful local socket write does not prove endpoint consumption.
The broader frozen media profiles and Gate 6 acceptance remain open.
