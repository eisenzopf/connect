# Third-party source provenance

The build uses the pinned MIT-licensed rvoip and Parley sources recorded in
`dependencies/source-lock.json`. Their license files remain in `.deps/rvoip` and
`.deps/parley` and are included in local distributions. The source lock records every applied patch and its digest, including Parley's
conference changes, opt-in application/session/authority plumbing, bounded media
queues, encoded-video transport, terminal Session resource lifetime, bounded
terminal wire/event delivery under backpressure, and declared-SSRC receiver-report
and transport-wide congestion feedback binding, plus bounded native VP8
reordering before peer-budget sequence expansion and bounded WebRTC audio
ingress ordering, plus opt-in encoded-video QUIC loss-detection tolerance and sampled congestion diagnostics, BBR transport selection with recorded model-bound corrections, WebRTC video pressure ordering and shared audio/video synchronization identity, negotiated RTX receive restoration and feedback binding, and source-clock provenance with generation-fenced sender reports, and initialized remote-track callback/publication ordering, and authenticated sender-report/source-description activation on existing receiving tracks. These are modified pinned sources; an upstream revision alone
is not the complete
build identity. The current lock lists 28 recorded patches, with source-lock
SHA-256 `dae61d19fc49a97a42912794ce2d9e33fb81a06e5a04135443c148a45cee78d0`.
Recheck the lock and patch digests when cutting a new source archive.

The pinned `quinn-proto` 0.11.19 crate is reproduced from its SHA-256-verified
registry archive with the recorded BBR model correction. Its bundled MIT and
Apache-2.0 license texts are retained in runtime `licenses/prepared/`; its
archive identity, VCS revision and patch digest appear in the source lock and
prepared license index. Both workspaces select this exact prepared protocol
crate; the isolated upstream Quinn API version remains independently pinned.

The `rtc-dtls` and `rtc-interceptor` 0.20.0-alpha.1 registry archives are also
reproduced by digest with recorded incomplete-flight retransmission and
receiver-report late-packet accounting repairs, RTX receive normalization and explicit source-clock sender reports. Their `MIT OR Apache-2.0`
license declaration and archive VCS provenance identify the `webrtc-rs/rtc`
revision in the source lock. The published archives omit license text; the
applicable root `LICENSE-MIT` and `LICENSE-APACHE` files from that exact revision
are retained under `dependencies/licenses/rtc`, with source URLs and SHA-256
values in the lock. Preparation verifies those texts, and runtime bundles copy
them with the patched crate provenance into `licenses/prepared/` and
`licenses/prepared-index.json`.

Thelve source is not incorporated. Bridgefu informs the request binding design;
no Bridgefu source is copied at this gate. Cargo.lock records registry dependencies.

Local runtime bundles include registry dependency license/notice texts and a
`licenses/registry-index.json` listing names, versions, declared licenses and
source registries. This index uses the full resolved workspace graph, a
conservative superset of the runtime binaries (Gate 5: `connect`,
`connect-example` and `connect-video`). It does not imply every
listed test/build dependency is linked into the server.

The independent UCTP control client uses separately prepared, hash-pinned Python
distributions; [its dependency notices](examples/independent-uctp-client/DEPENDENCIES.md)
record their declared licenses. Those installed environments are not copied into
the Connect runtime. FFmpeg, Chrome, Playwright and Node are selected external
prerequisites and are not bundled. Their license/build options are not replaced
by Connect's MIT license. No claim about an unbuilt candidate's completed notice
inventory is made by this source document.
