# Third-party source provenance

The build uses the pinned MIT-licensed rvoip and Parley sources recorded in
`dependencies/source-lock.json`. Their license files remain in `.deps/rvoip` and
`.deps/parley` and are included in local distributions. Every applied patch and
its digest is recorded in the source lock; an upstream revision alone is not the
complete build identity.

The current patch inventory and complete source-lock digest are recorded in the
machine-readable lock. The patches cover application/session authority, bounded
media and lifecycle handling, encoded-video transport, congestion and receiver
feedback, RTP repair, source-clock provenance and Gate 6 transport/lifetime
repairs. Verify the recorded digests when preparing a build; an earlier gate's
qualified source snapshot is not the current patch inventory.

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

The `web-transport-quinn` 0.11.12 and `web-transport-trait` 0.3.7 registry
archives declare `MIT OR Apache-2.0` and omit the repository-root license texts.
Their archive VCS metadata selects `moq-dev/web-transport` revisions
`4326b94ac251039964d35e9fe6490d15a14f8f66` and
`af182f99b7ca23482e765b09e70f274295a98813`, respectively. The complete root
`LICENSE-MIT` and `LICENSE-APACHE` files are retained under
`dependencies/licenses/web-transport-quinn/` and
`dependencies/licenses/web-transport-trait/`. Their immutable source URLs,
crate checksums, VCS paths and license SHA-256 values are recorded in
`dependencies/registry-license-lock.json`; package assembly verifies the archive
and license provenance before copying these notices.

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
