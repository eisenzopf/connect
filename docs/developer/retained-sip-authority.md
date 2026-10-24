# Retained SIP consultation authority

This component adds a typed SIP retained leg to consultation planning. The private
pair still uses native Connections. Actual retained SIP network qualification and
selected/private SIP endpoints remain pending.

`ConsultationMediaOrigin` carries either native metadata or the actual initial or
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

The [component report](../evidence/gate-6/auxiliary-retained-sip-authority.json)
records 167 distinct controlled Store cases and the service compile check, plus
failed intermediate attempts and remaining actual-wire work. These checks do not
close either DTMF clause. The existing transition wire contract and
[canonical HTTP reference](../../contracts/openapi-canonical-http.json) are unchanged.

The initial retained-call fixture is
`production_initial_sip_retained_consultation_fences_dtmf_and_restores_original_call`
in the `gate6_canonical_runtime` server test target. It uses actual local SIP
signaling/RTP and signed native clients for both EdDSA and ES256. Its assertions
cover private audio exclusion, held SIP input settlement, private native DTMF,
return/replay, fresh RFC4733 delivery and shutdown of the original call. Adding or
compiling the fixture does not qualify these behaviors. The
[fixture checkpoint](../evidence/gate-6/auxiliary-retained-sip-wire.json) records
its current verification state.

Set `CONNECT_RETAINED_SIP_OBSERVATIONS` to a fresh absolute JSONL path physically
under this checkout's ignored `.local/gate-6/` directory to capture the actual
SDK-observed frames and operation summaries. Validate a completed capture with
`python3 scripts/validate-gate6-retained-sip.py <observations.jsonl>` in an
environment containing `jsonschema`. The validator checks wire/status schemas and
correlations; the SDK verifies signatures and the runtime fixture checks decoded
media, exclusion, replay and cleanup. This profile does not cover committed SIP
retained calls, private SIP endpoints, external carriers or other hosting profiles.

Return completion now distinguishes the original SIP call's physical presence
from its current receive authority. A live call stays in retained IDs even after
receive rights are withdrawn; `remote_continuing` also requires current named
receive rights and reciprocal recipient ceilings. Route-owner credentials apply
to the phone Participant's identity. The
[completion report](../evidence/gate-6/auxiliary-retained-sip-completion.json)
preserves the actual return failure, controlled reproductions, fix and current
component checks. The corrected actual-wire rerun remains separate.
