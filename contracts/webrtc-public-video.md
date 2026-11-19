# Original public video B wire and HTTP status contract

This is the closed optional WebRTC target contract. Implementation and runtime
qualification remain separately recorded in Gate 6; this reference grants no
media, native custody or publication permission.

`webrtc_video_receive_reservation` is the original server-created Intent:
`profile: connect/webrtc-public-video-reservation/1`, `phase: intent`,
`reservation_id`, `nonce`, `preview_stream_id`, `preview_role: receive`, and
`public_stream_id`. A and B IDs are distinct and match the one frozen original
A publication/B receiver mapping. No transport identity, cut, readiness count,
JS track ID or authority field is allowed in Intent. The supported target uses
three roles: audio sendrecv, A sendrecv (camera plus private preview), B recvonly.
The original actual peer/offer owns all three MIDs; echo alone enables nothing.

Current-authorized original `operation.get` may expose that Intent plus
`webrtc_public_video_phase`. The closed phase profile is
`connect/webrtc-public-video-phase/1`. Every phase carries original reservation,
operation record/logical operation, target connection/generation and binding.
The binding fields are `stream_id`, `preview_stream_id`, actual ASCII `mid`,
`stream_generation`, opaque `physical_writer_incarnation`, independent
`connection_generation`, `primary_ssrc` (zero valid), `payload_type` (0..127),
`clock_rate_hz: 90000`, `codec: vp8`. Native pointers/owners are checked separately.

| Phase | Additional closed fields | Required original runtime boundary |
| --- | --- | --- |
| `negotiated-never-fed` | none | Actual original reserved B binding, never fed |
| `cut-bound-post-f` | opaque `source_cut_id`, `public_readiness` | Same OLD F and guarded original B warm owner |
| `decoded-ready` | same cut and challenge | Actual successful local-native submission receipt for the SAME whole post-F keyframe, plus the same signed B decoded observation and retained original lease |
| `activated` | same cut and challenge | Same writer adopted and phase appended in the existing Completed SQL/installation commit barrier |
| `discarded` | required `source_cut_id` string or null, `reason` | Actual whole original physical/backend/Core joins under exact historical cleanup custody |

`public_readiness` contains only its original nonce and
`decoder_startup: fresh-receiver-first-decoded-frame`. Intent, private and public
nonces are distinct. Generations are independent namespaces, not arithmetic
aliases. Wire cut IDs are nonempty bounded visible ASCII strings; the original
realm/u64 F never becomes a JSON number. Phase replay preserves every immutable
ID/binding/cut/challenge. State/shape alone cannot supply the required owners.

Before negotiated B binding, terminal cleanup emits no wire Phase or invented
MID/SSRC. A known physical move that stopped may settle as stopped without
Activated; it is nonrenderable. Completed replay never repairs a missing
Activated with a later append. Discarded records cleanup only and cannot thaw
OLD; actual same Closed token and fresh OLD/every selected publisher authority
are separate requirements. A typed owned NotSubmitted capture may support
cleanup only after its exact durable pre-Core negative is verified; raw failed
or actual CommitUnknown rows cannot substitute.

The public acknowledgment is a closed `replacement_preparation` marker in the
SAME original signed `connection.offer`: `profile:
connect-replacement-public-video-ready-v1`, `operation_record_id`,
`public_video_readiness`. The latter has original `reservation_id`,
`source_cut_id`, public `nonce`, exact `binding`, positive `decoded_frames`,
required `decoded_keyframes` positive or null, and the startup literal above.
A present key count must not exceed frames; absent native key counters require
explicit null. Audio/private A evidence is separate. Every other frozen offer,
signer/key/device/credential/scope field remains identical. Host compares the
whole original captured marker, including counts, before replay; a Store result
observation does not reconstruct capture or grant media or renew expiry.

HTTP exposes current-authorized original status/cancel/recovery only. HTTP
`connection.offer` is 501 unsupported, and no HTTP payload acquires physical
readiness/custody. See [the schema source](schemas/webrtc-public-video-1.schema.json)
and `WebRtcPublicVideo*` components in the canonical HTTP reference. The schema
checks closed structure and safe scalar bounds. Runtime additionally enforces
original cross-field equality, distinct IDs/nonces, key-count relation, actual
native receiver/writer ownership, current authority and publication/close proofs.
