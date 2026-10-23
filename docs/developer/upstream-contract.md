# Gate 6 reusable upstream contract

G6-028 qualifies the combined current M1 engine contract before downstream
installation is called complete. Its current qualification status is recorded in [the combined evidence](../evidence/gate-6/upstream.json) and the implementation plan. Component reports retain their own source scope and actual execution counts; adding their historical totals does not qualify a new graph.

`dependencies/source-lock.json` is authoritative for source revisions, ordered
patches, digests and third-party license provenance. RVoIP is pinned to
`ca7861af4c9920f6949422ae1f40507bf56ede97` plus the recorded patches; Parley
`22509d4df4cbf8dccd2ac7d35d0ac3becc724ceb` is reference material. The recorded
RTC and Quinn sources retain their separate upstream revisions and licenses.
Prepared `.deps` source is generated, and unpublished changes are consumed from
recorded patch bytes rather than an unmerged upstream contribution.

| Engine boundary | Contract / public integration guidance |
|---|---|
| Wire decoding and canonical dispatch | [Connect UCTP profile](../../contracts/connect-uctp.md), published envelope schema and vectors; typed UCTP handlers retain current physical-peer ownership. |
| Identity, membership and addressed messaging | [Engine model](../decisions/013-gate-6-model.md), [addressed engine](../decisions/017-gate-6-addressed-engine.md); host policy owns durable audience decisions. |
| Negotiation, stream changes and lifecycle | [Native negotiation](../../contracts/native-media-profile.md), [codec updates](../../contracts/native-codec-profile.md), [native lifecycle](../../contracts/native-lifecycle-profile.md). Preparation, confirmation, permitted flow and application presence remain distinct facts. |
| Managed scene, consultation, handoff and replacement | [Media scene](media-scene.md), [private consultation](../../contracts/private-consultation.md), [handoff](../../contracts/connection-handoff.md), [device replacement](../../contracts/device-replacement.md). Use retained IDs, expected revisions and explicit current host policy. |
| Framing, codecs, feedback, DTMF and quality | [Media profiles](media-profiles.md), [received DTMF](received-dtmf.md), [signal profile](../../contracts/native-signals-profile.md), [fallback audio](websocket-media.md). Physical observations preserve original timing and method; local IO success is not remote consumption. |
| Provider-neutral observations | [Agent observations](../../contracts/agent-observations-profile.md); provider payloads stay inside adapters. Intent is not permission or completed media movement. |
| Authentication and assurance | [Connect UCTP profile](../../contracts/connect-uctp.md); signed invocation custody, physical peer, credential lifetime and current policy are checked through actual IO. |
| Recording and vCon | [Conversation records](../../contracts/connect-records.md); retain completeness, source provenance, redaction and lifecycle rather than inventing missing provider facts. |

The combined qualification procedure prepares exact sources in a fresh destination
and compares them with the consumed tree, runs a focused shared graph covering
protocol, media, participant/session lifecycle, authentication and record/recovery
behavior, then probes the locked Connect consumer. Source, patch, executable and
log digests must identify that actual run. A skipped or unresolved check is not
a pass. Public API documentation and patch provenance accompany the resulting
`docs/evidence/gate-6/upstream.json` only after those checks finish.

G6-028 does not replace production canonical Store/journal/service/HTTP/SDK
installation, later native/browser consumer work, Linux hosting, independent
conformance, extended operating/performance/impairment profiles, or bounded named
remote/provider rehearsals in G6-093. Those remain assigned plan tasks. The
journal is authoritative and events are hints; current authorization is required
on replay. Publication remains a separate action.
