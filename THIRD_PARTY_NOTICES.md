# Third-party source provenance

The build uses the pinned MIT-licensed rvoip and Parley sources recorded in
`dependencies/source-lock.json`. Their license files remain in `.deps/rvoip` and
`.deps/parley` and are included in local distributions. Connect applies Parley's
recorded conference patch and a small opt-in QUIC application-handler patch.

Thelve source is not incorporated. Bridgefu informs the request binding design;
no Bridgefu source is copied at this gate. Cargo.lock records registry dependencies.
