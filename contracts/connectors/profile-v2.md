# Connect connector contract v2

Manifest version `2` and the `connect-connector-fixture-v2` report identify the
current public `connect-connector-api` contract. Version 1 manifests and the old
`audio`/`video` fields are refused. There is no downgrade path. Persisted v1
registrations need an explicit owner-managed migration or fresh pre-alpha state;
startup must not silently reinterpret their capabilities.

Trusted Rust composition selects implementations and resolves configuration.
There is no wire operation to load code, register a manifest, provision an
Endpoint or acquire conversation authority. Configuration contains only validated
`env:NAME` or `secret:opaque-key` references. Resolved values remain private.
Registration records an immutable declaration; it does not establish remote
availability, media readiness, decoded speech, endpoint confirmation or consent.

## One explicit media classification

`Manifest.media` is a closed tagged union:

- `unsupported`: messages or telephone control only; no media installation.
- `lifecycle_fixture`: bounded mono signed 16-bit PCM at 20 ms. The fixture's
  open/reconcile/close/exchange exercise is not a transport registration.
- `live`: a declared transport or signed native Endpoint host, with explicit
  codecs, controls, context handling and record handling.

The typed operation set must agree with the classification. `MediaExchange`
belongs only to the lifecycle fixture. `LiveMedia` belongs only to a live
binding. SMS cannot acquire either media classification or voice control.
Messaging, telephone control and native assistant hosting are separate profiles.
Vapi chat does not imply native audio, and telephone API control does not install
SIP media. The canonical Vapi native host is an enrolled native Endpoint using
the public SDK; it is not a registration of the rvoip Vapi transport adapter.

`LiveBinding.transport` is the exact rvoip Transport serialization supplied by
trusted composition. The host derives `MediaRuntime` from the actual existing
`ConnectionAdapter`. The public validation policy compares binding, codec
families and lifecycle guarantees without branching on a provider name.
A signed native host has its own binding and still requires current Endpoint,
Device/key, credential, audience, source and owned lifecycle checks.

`LiveCodecs.advertised` contains bounded unique audio, video and data families.
Audio identifies name, RTP clock and channels; video identifies name and RTP
clock. Format parameters, packetization, stream selection and physical readiness
remain negotiated per Connection. An empty runtime descriptor is explicitly
`negotiated`, rather than a fabricated default codec. A negotiated-only profile
returns `MediaNegotiation` to static capability requests. No declared format may
expand the actual runtime's advertised set.

Controls are explicit: end, authenticated inbound admission, staged origination
and device replacement. All live profiles require end support. Current runtime
liveness and terminal fallback are mandatory. Inbound admission additionally
requires atomic authenticated handoff; staged origination and device replacement
require their actual runtime guarantees. Core registration and every control
operation still require the host's normal policy and resource checks.

`context:host_selected` permits only bounded historical sources independently
selected by current host policy. It must agree with the manifest's context and
`ContextExport` declarations. These sources remain data with provenance, not
instructions or new authority. `records:host_authorized_streams` describes host
capture of selected streams under current audience/retention/link policy; it does
not authorize recording or expose a provider transcript by itself.

## Normalized effects and recovery

Messages retain original mutation IDs, input, separate authorized context and
attributed result sources. Provider acceptance is distinct from recipient
delivery. Typed voice and message observations retain logical/external/event IDs
and authenticated verification provenance. Incoming callbacks require exact-byte
source authentication before normalization. Caller ID and external identifiers
never establish a principal or merge histories.

Hosts check current authorization before replay or duplicate application and
persist projections with effects and journal facts. Uncertain creation is
reconciled under the original identity and full original binding; an absent,
ambiguous or truncated lookup permits no resubmission. Contradictory terminal
observations require reconciliation. Adapters make one effect attempt under
current protected host custody. Teardown, cancellation and recovery remain owned
and bounded; dropping a handle requests cleanup rather than proving it.

The lifecycle fixture harness retains independent provider-state persistence,
signed callback tests, reordered/duplicate events, uncertainty, cancellation,
exactly-once logical effects, bounded PCM exchange and paired teardown. Its report
explicitly excludes live provider, network media and video qualification. A real
transport additionally needs the existing rvoip adapter interfaces, trusted host
installation and the assigned current Connect runtime/profile evidence.
