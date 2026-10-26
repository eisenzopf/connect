# Native codec controls and Session retirement

Signed QUIC `connection.update` with `action: "codec-renegotiate"` changes
selected streams on the original physical native Connection. It requires current
`connection.update` permission, the original Device/key/credential/member epoch,
current accepted Session and verified endpoint/profile capabilities, and the
exact current stream revision. Offer or subscription permission does not grant
codec changes. HTTP returns 501 because it has no native physical custody;
current operation status and cancellation remain available there.

The request retains original preference order, optional explicit stream targets,
revision and allowed extensions in its durable intent. A new or still Accepted
root must prove the exact original physical peer before admission or mutation.
A foreign physical peer cannot reserve a new operation ID or fail an Accepted
original root. Completed or uncertain replay is current-authorized historical
metadata and never invokes Core again.

One durable attempt precedes the native codec update. The held plan checks
current source and target readers and exact selected media. A signed counteroffer
must be confirmed with the exact codec/direction/handle set; a stale confirmation
leaves the operation preparing and old authorized audio continues. Unsupported
codecs, stale revisions and unknown stream targets refuse without a native effect.
Hold/resume, mute/unmute and quality controls have separate runtime paths;
see `native-runtime-controls.md`.

Only an actual successful native return creates a captured codec receipt. Its
`codec_bindings` identify actual negotiated medium, direction, format, native
handle, stream generation and Connection generation. These are observations,
not new media grants. RTP hop handles are positive and unique in a captured
set. Reliable data has no RTP hop and retains the reserved value `0`; multiple
distinct logical data streams can share that reserved observation value. Codec
replacement of selected RTP streams preserves those unchanged data bindings.
The HTTP reference names these historical shapes `NativeCodecBinding` and
`NativeCodecResult`. Completion advances the stream revision and updates
readiness only for the original still-live generation. Historical completion
cannot revive a retired slot. The current schema 112 retains this command in
the native control ledger; old pre-alpha databases are refused.

Recording retry retains only the captured receipt and Store kernel, with no
command, readers or physical peer. A public role writer can proceed while
storage is unavailable. Late cancellation preserves uncertainty; recovery records
the actual result under the original root and effect without reapplying Core.
Restart retains unknown work and rejects predecessor-runtime receipts.

Session retirement uses the durable aggregate state: ended, rejected, cancelled
and failed are terminal. It does not infer termination merely from a party's
reject action while other participants remain. Both signed and HTTP terminal
controls retire the native runtime. A persistent Conversation remains open after
its main Session ends, may acquire a successor main Session, and closes only
through its explicit policy after active work has ended.

Local production tests exchange real signed QUIC control and verify PCMU/PCMA
media in both directions with independent FFmpeg decoding. They cover stale
confirmation with old audio preserved, actual new negotiated handles, idempotent
replay, recording failure with a concurrent role writer and HTTP cancellation,
current refusal, and foreign physical requests preserving the pending original.
Separate controlled Store tests cover intent identity, separate permission,
recording rollback, late retirement and restart uncertainty. The main-Session
case covers actual runtime cleanup, successor cancellation and HTTP rejection,
plus explicit Conversation closure. These local macOS arm64 checks do not qualify
Linux/browser/remote/provider profiles or Gate 6 acceptance.
