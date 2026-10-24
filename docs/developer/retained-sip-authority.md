# Retained SIP consultation authority

Consultation can retain an initial or committed SIP call while the private pair
uses native Connections. Both local profiles pass for EdDSA and ES256, including
private audio/DTMF exclusion, original-call return/replay, fresh RFC4733 delivery
and owned cleanup. The [private SIP-primary profile](private-sip-primary-store.md)
also has actual initial/committed-call evidence. SIP-candidate admission and the
remaining authority and retirement compositions remain pending.

`ConsultationMediaOrigin` carries native metadata or the actual initial or
committed SIP origin. A saved SIP snapshot is matched to a previously observed
metadata origin in the current runtime and checked against fresh complete facts.
The readiness registry keeps no authority reader and cannot grant IO. Missing,
foreign, changed, expired or retired sources refuse; SIP is never represented by
invented native signing fields.

Planning and execution acquire the full current native and SIP footprint. A
captured outcome updates only the original SIP call generation and receiver
revision. Return checks SIP-target/native-source permission separately;
native receive permission cannot stand in for the phone's receive permission.
The held control also checks parent expiry, cancellation, source identity and
activation. Historical departure proves exact recorded absence and cannot
restore another generation or authorize a new call.

Completion distinguishes physical presence from current receive authority. A
live call stays in retained IDs after receive rights are withdrawn;
`remote_continuing` also requires current named receive permission and reciprocal
recipient ceilings. Route-owner credentials apply to the phone Participant's
identity. Completion grants no media reader or IO, and waits for actual candidate
retirement before recording its result once.

A completed telephone move reports the new SIP Connection in
`operation.result["connection_id"]`. `operation.connection_id` continues to name
the replaced native request source, which is retired after the real commit.
Use the result ID for a later retained-call command and preserve both IDs when
showing the move. The committed fixture asserts their separation, the current
generation2 SIP lifetime, original source/controller revocation and expired
preparation before entering consultation.

The [pair report](../evidence/gate-6/auxiliary-retained-sip-pair.json) records one
unchanged optimized executable: initial SIP31.53s, committed SIP54.74s, four
signing scenes and twelve validated actual SDK-observed frames. It preserves
the first committed fixture's correctly refused retired-source attempt. The
[completion report](../evidence/gate-6/auxiliary-retained-sip-completion.json)
records 171 distinct affected Store cases and service compilation. The
[earlier initial report](../evidence/gate-6/auxiliary-retained-sip-wire.json) and
[authority report](../evidence/gate-6/auxiliary-retained-sip-authority.json) retain
their historical source scopes. These results do not close either complete DTMF
clause or the whole task.

Both entry points in the `gate6_canonical_runtime` server target use the same
exercise in `telephone/retained.rs`:

- `production_initial_sip_retained_consultation_fences_dtmf_and_restores_original_call`
- `production_moved_sip_retained_consultation_fences_dtmf_and_restores_original_call`

They use actual loopback SIP/RTP and signed native public clients. Set
`CONNECT_RETAINED_SIP_OBSERVATIONS` to a fresh absolute JSONL path physically
under this checkout's ignored `.local/gate-6/` directory for each profile.
Validate the initial capture with
`python3 scripts/validate-gate6-retained-sip.py <observations.jsonl>`; add
`--source-kind established-sip` for the committed capture. The environment needs
`jsonschema`. The SDK verifies signatures, the validator checks schemas and
correlations, and the runtime checks media, exclusion, replay and cleanup.
External carriers and other hosting profiles require separate proof.

The transition wire contract and
[canonical HTTP reference](../../contracts/openapi-canonical-http.json) are unchanged.

The separate [private SIP-primary Store component](private-sip-primary-store.md)
now has typed protected planning and current-context checks. It does not yet
qualify a SIP endpoint as an actual private runtime participant.
