# Third-party source provenance

The build uses the pinned MIT-licensed rvoip and Parley sources recorded in
`dependencies/source-lock.json`. Their license files remain in `.deps/rvoip` and
`.deps/parley` and are included in local distributions. The source lock records every applied patch and its digest, including Parley's
conference changes, opt-in application/session/authority plumbing, bounded media
queues, encoded-video transport, terminal Session resource lifetime, bounded
terminal wire/event delivery under backpressure, and declared-SSRC receiver-report
and transport-wide congestion feedback binding. These
are modified pinned sources; an upstream revision alone is not the complete
build identity. The current lock lists 13 recorded patches, with source-lock
SHA-256 `a391f1650752107987b2f5c7c30cc7f653d372ccf65092336d2aed80d6d37d6a`.
Recheck the lock and patch digests when cutting a new source archive.

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
