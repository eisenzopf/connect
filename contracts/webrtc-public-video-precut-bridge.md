# Optional signed WebRTC B pre-cut bridge

This explicit profile extends the original public-video operation without changing
v1 acceptance when absent. Schema/source statements are not runtime evidence.

The original authenticated `connection.replace` payload may include this immutable
extension, retained in its original operation digest:

```json
{"receiver_video_bridge":{"profile":"connect/webrtc-public-video-precut-bridge/1","carrier":"webrtc-rtp","presentation":"exclusive-public","feed":"public-programme-private-decode"}}
```

It requires exactly one A publication/B receiver mapping, an ordinary public source
and WebRTC target. The SAME signed target initial offer must echo that closed
object under `webrtc_public_video_bridge_opt_in`, alongside the original exact
`webrtc_video_receive_reservation`. Neither echo enables IO or grants a route.
The choice is inserted atomically with the original confirmed Prepare/reservation;
late opt-in, missing or foreign target echo and non-WebRTC transport are refused.

Current-authorized `operation.get` may expose `webrtc_public_video_bridge` with
`opt_in`, nullable `feeding`, and nullable `decoded_ready`. `feeding` binds the
original reservation/logical operation/record, target connection/generation and
exact negotiated B binding. `bridge_readiness` contains one server nonce,
`decoder_startup: fresh-b-receiver-precut-keyframe`, and absolute Unix-second
`expires_at`. Store selects its cutoff once, at most five seconds from that
first Feeding commit and no later than original reservation/current source
expiry. Replays retain the SAME nonce/cutoff, never renew them.

The target's signed `connection.offer` repeats its whole original immutable offer
and uses `replacement_preparation` with exactly `profile`, `operation_record_id`
and `bridge_readiness`. Its profile is
`connect-replacement-public-video-bridge-ready-v1`; the observation contains
`reservation_id`, `nonce`, `binding`, exact `decoded_rtp_timestamp` (u32), positive
JS-safe `decoded_frames`, and the same `decoder_startup`. B must be the actual
original receiver; a wall clock, A preview, frame counter or JS track ID cannot
replace the native RTP coordinate. The trusted Host guard joins this signed
coordinate to an independently observed successful complete contiguous VP8
keyframe on SAME B physical writer/generations/SSRC/PT before committing
`decoded-ready`. Changed repeated observations conflict. A repeated reply is a
current-authorized read of the exact captured observation, not another grant.

The append-only bridge chain is separate from the unchanged v1 public-video
phase chain. It contains no F, `source_cut_id`, CutBound, post-F Ready, Activated,
public presentation, OLD retirement or Commit. Existing post-F final readiness,
current policy and physical IO checks remain mandatory. No fallback activates a
normal route, private A, foreign writer or a non-opted receiver.

HTTP current-authorized status reads expose these optional facts. HTTP
`connection.offer` stays unsupported (501): only the actual signed target physical
writer can send this Ready. The existing canonical HTTP/OpenAPI source references
both this document and the closed schema. Regeneration and runtime controls are
separate owner actions; they have not been run for this ignored draft.
