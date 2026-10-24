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
records167 distinct controlled Store cases and the service compile check, plus
failed intermediate attempts and remaining actual-wire work. These checks do not
close either DTMF clause. The existing transition wire contract and
[canonical HTTP reference](../../contracts/openapi-canonical-http.json) are unchanged.
