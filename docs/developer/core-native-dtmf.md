# Core delivery to native DTMF recipients (G6-024 in progress)

Core selects native signaling for a negotiated QUIC/WebTransport recipient, independently of the requested gateway method. `DtmfRoutePolicy::acquire_native` receives the immutable original operation/intent and source observation, exact Core source/recipient endpoints, the native recipient's actual identity/device/endpoint/member epoch, credential and stream revisions, physical credential incarnation, negotiated audio bindings and actual signed envelope bytes. RTP and INFO permission callbacks cannot substitute for this native signal grant. An unconfigured recipient refuses without raw dispatch or a different writer.

The actual authenticated recipient handler explicitly registers its DTMF writer after signed media negotiation. The native adapter retains a weak reference in the exact connection route. Replacing the registration retires only the old registration; a queued old delivery cannot borrow its successor. The same canonical actor signs and frames `dtmf.received` for the recipient's own wire Conversation/Session/Connection scope. The original logical operation remains unchanged.

The bounded queue retains only intent, source/target metadata, policy callback and caller cancellation/deadline. It retains no permission reader. After its actual dequeue, the sole physical writer obtains fresh independent source/recipient signal authority for the signed bytes, within the existing 200 ms acquisition/write bound. Native queue/writer submission also remains within one second and the original caller/credential deadlines. Store/journal deduplication and replay authorization remain the host's responsibility.

A target metadata fence surrounds the Core/source IO fences. It tries the actual native flow/controller/credential/audio-port locks and validates the original epoch, revisions and active exact handles. It releases all synchronous locks on Pending. Contention defers IO within the original bound. Source/recipient retirement, replacement or credential changes cannot turn old queued metadata into permission. No original `AuthorizedEnvelope` is retained as authority for a later operation.

The owned write future takes the actual framed native sink only after all physical IO fences admit its poll. A definite refusal before that poll leaves the healthy writer available. Once IO may have started, cancellation, failure or expiry is `OutcomeUnknown`; the sink is destroyed before its actual permission guard is released, and the possibly partial reliable stream is retired. Dropping the entire caller-owned future follows the same custody order. Already admitted bytes cannot be recalled. Success proves local signed reliable-writer submission, not remote receipt or tone playback; ordinary transport recovery does not create another logical operation.

`DtmfSourceObservation` separates actual source kind, stable physical observation ID and receive time from the requested gateway method. Native ingress derives these fields from its accepted signed envelope and sealed physical source. A direct application command uses `application` provenance and no fabricated packet observation. Trusted received-source composition must attach actual source IO custody; payload claims cannot supply it. Enriched received SIP/WebRTC composition remains separate work.

The source-scoped checker uses real local signed TLS QUIC/WebTransport recipients and actual SIP/SDP source connections, plus native-to-native delivery through two managed native connections. It tests both requested gateway methods, exact authorized/received signed bytes, original operation/provenance, queued revocation without readers, drop/cancel/deadline, recipient key/rights changes, source retirement, target stream retirement after fresh permission, obsolete reoffer/registration and missing configuration. Controlled sink tests cover Pending IO destruction before guard release and preservation of the healthy writer on definite pre-IO refusal. A retained pre-fix executable reproduces configured native recipients stopping at unsupported transport dispatch.

Full G6-024 remains open for enriched received SIP/WebRTC attribution and remaining approved media framing/size/fragmentation/late/malformed/feedback/decoded profiles. Production Store/journal/service/HTTP/SDK installation and Linux/remote/browser/provider qualification are not established by these local controlled policy fixtures.

## Managed original and current audience

Connect's G6-046 integration now has an explicit managed-source primitive.
`receive_managed_native_dtmf` captures `NativeDtmfDisclosure` from the actual
managed Core source/controller before accepting its signed observation. It
retains the original scene groups, exact source incarnation and credential
metadata, with no original envelope or permission reader. Capture changes no
capability declaration, port or media programme. The general ingress primitive
continues to require its host's own explicit audience composition; a failed
managed capture does not fall back to it.

For a native recipient in the same managed Core Session, the route policy can
call `native_dtmf_audience_fence` with that source disclosure and the actual
`DtmfNativeWriteScope.recipient`. The policy must first authorize the immutable
original source operation and this exact current recipient, then attach the
returned fence to its fresh `DtmfWriteLease`. The fence binds the recipient's
actual controller, identity, Device, Endpoint, Participant epoch, principal and
credential revision. A payload claim cannot construct the source disclosure.

During physical IO, Core's route and the sealed native source/target fences
still retain their exact registry, lifecycle, credential, flow and audio-port
custody. The additional audience fence only tries the scene member/audience
registries and checks original plus current boundaries. It does not reacquire
controller/credential or lifecycle locks already held around that poll, or call
Store/policy. Pending releases all its guards. A private boundary remains in the
original disclosure after a later return removes it from the current scene;
another Connection of the same Participant does not inherit membership.

The three new controlled scene tests cover nested physical controller fences,
Pending guard release, exact recipient binding and private return. Together with
both existing capability regressions and 30 actual native/SIP DTMF cases, 35
focused engine checks pass. The signed native-to-native QUIC/WebTransport case
now uses managed source capture and the audience fence during fresh native
writer authorization. The locked offline Connect server compile check passes.
[The scoped evidence](../evidence/gate-6/auxiliary-native-dtmf-audience.json)
records the failed disk-capacity build and unchanged-source recovery as well.

The production Connect Store/service/SDK/HTTP composition remains required.
This same-Session native helper does not establish SIP/WebRTC audience
composition, ES256 runtime in this phase, whole Connect private-tail recovery,
Linux/remote/browser/provider qualification or remote playback.
