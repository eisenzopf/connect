# Third-party source provenance

The build uses the pinned MIT-licensed rvoip and Parley sources recorded in
`dependencies/source-lock.json`. Their license files remain in `.deps/rvoip` and
`.deps/parley` and are included in local distributions. Every applied patch and
its digest is recorded in the source lock; an upstream revision alone is not the
complete build identity.

The current lock lists 34 recorded patches, with source-lock SHA-256
`76b0047d18f265221798b50ce91cdb728b2301533a453e6dfa4841b4c515cd71`.
The patches cover application/session authority, bounded media and lifecycle
handling, encoded-video transport, congestion and receiver feedback, RTP repair,
and source-clock provenance. The latest media repairs handle negotiated outgoing
RTX, first authenticated audio feedback, and initial sender reports arriving
before compatible source descriptions. The Parley qualification patch explicitly
selects the prepared dependency sources and preserves absent remote-clock
provenance for locally synthesized prompts. Recheck the lock and patch digests
when cutting a new source archive.

The pinned `quinn-proto` 0.11.19 crate is reproduced from its SHA-256-verified
registry archive with the recorded BBR model correction. Its bundled MIT and
Apache-2.0 license texts are retained in runtime `licenses/prepared/`; its
archive identity, VCS revision and patch digest appear in the source lock and
prepared license index. Connect, Rvoip and standalone Parley qualification select this exact prepared
protocol crate; the isolated upstream Quinn API version remains independently pinned.

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
