# Retained SIP consultation authority

Consultation planning can retain an initial or committed SIP call while the
private pair uses native Connections. The actual local initial-call profile now
passes for both EdDSA and ES256, including private audio/DTMF exclusion,
original-call return/replay, fresh RFC4733 delivery and owned cleanup. Committed
retained calls and selected/private SIP endpoints remain pending.

`ConsultationMediaOrigin` carries native metadata or the actual initial or
committed SIP origin. A saved SIP snapshot is matched to a previously observed
metadata origin in the current runtime and checked against fresh complete facts.
The readiness registry keeps no authority reader and cannot grant IO. Missing,
foreign, changed, expired or retired sources refuse; SIP is never represented by
invented native signing fields.

Planning and execution acquire the full current native and SIP footprint. A
captured outcome updates only the original SIP call generation and receiver
revision. Return uses separately checked SIP-target/native-source permission;
native receive permission cannot stand in for the phone's receive permission.
The held control also checks the parent interval, cancellation, source identity
and activation. Historical departure proves only exact recorded absence, and
cannot authorize a new call or restore a different generation.

Completion distinguishes physical presence from current receive authority. A
live call stays in retained IDs after receive rights are withdrawn;
`remote_continuing` also requires current named receive permission and reciprocal
recipient ceilings. Route-owner credentials apply to the phone Participant's
identity. Completion grants no media reader or IO, and still waits for actual
candidate retirement before recording its result once.

The [initial wire report](../evidence/gate-6/auxiliary-retained-sip-wire.json)
records the unchanged-source 28.34-second runtime and six validated actual
SDK-observed frames. The
[completion report](../evidence/gate-6/auxiliary-retained-sip-completion.json)
records 171 distinct affected Store cases and service compilation. The
[earlier authority report](../evidence/gate-6/auxiliary-retained-sip-authority.json)
retains its original component scope. Failed, interrupted and diagnostic runs
remain recorded; these reports do not close either complete DTMF clause.

The test is
`production_initial_sip_retained_consultation_fences_dtmf_and_restores_original_call`
in the `gate6_canonical_runtime` server target. It uses actual loopback SIP/RTP
and signed native public clients. Set `CONNECT_RETAINED_SIP_OBSERVATIONS` to a
fresh absolute JSONL path physically under this checkout's ignored
`.local/gate-6/` directory to capture the actual frames and operation summaries.
Validate a completed capture with
`python3 scripts/validate-gate6-retained-sip.py <observations.jsonl>` in an
environment containing `jsonschema`. The SDK verifies signatures; the validator
checks schemas and correlations; the runtime checks media, exclusion, replay
and cleanup. External carriers and other hosting profiles require separate proof.

The transition wire contract and
[canonical HTTP reference](../../contracts/openapi-canonical-http.json) are unchanged.

The initial and committed retained-call entry points now use one shared exercise
in `telephone/retained.rs`. Both entry points compile, and the committed setup
asserts the original controller/source revocation and expired preparation.
The [pair report](../evidence/gate-6/auxiliary-retained-sip-pair.json) tracks the
new source; its actual runtime is pending. For committed captures, pass
`--source-kind established-sip` to `validate-gate6-retained-sip.py`.

A completed telephone move reports the new SIP Connection in
`operation.result["connection_id"]`. `operation.connection_id` continues to name
the replaced native request source, which is retired after the real commit.
Use the result ID for a later retained-call command; preserve both IDs when
showing the move. The committed fixture asserts their separation and the actual
generation2 lifetime. Its first attempt correctly refused the old source and is
preserved in the pair report; corrected-pair runtime verification is pending.
