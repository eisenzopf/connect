# 008 — Autonomous live audio qualification

The owner requested on October 9, 2026: “please do e3e testing on your own. you
should be albe to do your own testing without me.” The owner also confirmed the
browser conversation worked. Gate 3 retains its transport, three-turn,
interruption, teardown and continuity requirements. Independent decoding of real
returned audio may supply audibility evidence without requiring another human
rehearsal for every route.

Use generated speech through the public native or browser SDK, and an owned,
bounded telephone test peer. Verify answers against facts seeded only in Connect,
record spoken decisions, restart and ask for them without supplying the answers.
Local speech recognition of captured return audio is independent of provider
transcripts. For a telephone peer, its separately attributed received transcript
must agree with Connect's transcript and paired SIP/Vapi bindings. Provider
submission, start time, packets, sound energy or unchanged conversation IDs alone
are never audio or continuity passes.

An interruption must overlap actual returned speech; a fixed prompt delay does
not establish that. Require a truncated first response and successful handling
of the interrupting instruction. Test remote hangup separately from local
hangup and distinguish a normal media end from a timeout or uncertain transport.

This qualifies local macOS arm64 and the recorded network/carrier topology.
Telephone calls between owned Telnyx numbers qualify on-net E.164 telephone
routing; they do not attest an arbitrary off-net carrier. LAN WebRTC does not
attest WAN, TURN, mobile devices or audio hardware. Keep unsuccessful trials and
route restoration evidence. No required behavior is waived by this decision.
