# Isolated native remote fixture

Build the owned Ubuntu 24.04 Linux/amd64 SDK compiler image locally:

```sh
docker build --platform linux/amd64 -f scripts/fixtures/native-remote/Dockerfile.base -t connect-gate6-native-client:rust1971-ubuntu2404 scripts/fixtures/native-remote
docker build --platform linux/amd64 -f scripts/fixtures/native-remote/Dockerfile.verify -t connect-gate6-native-client:verified1971-ubuntu2404 scripts/fixtures/native-remote
docker build --platform linux/amd64 -f scripts/fixtures/native-remote/Dockerfile.clang -t connect-gate6-native-client:clang1971-ubuntu2404 scripts/fixtures/native-remote
```

The base reference pins the actual qualified Ubuntu image digest. Compiler and
package versions and the resulting image ID are recorded by each run. Clang is
used because GCC13 cc1 crashed under local x86 emulation in the retained initial
attempt. No dependency source or cryptographic verification is bypassed.

Run `scripts/rehearse-gate6-native-remote.py --help`. Supply a fresh output directory
under `.local/gate-6/native-remote`, a native release cache and a Linux build cache.
The build is locked/offline with the pinned prepared sources. Both signing
algorithms run through the public SDK from a separate Ubuntu guest to the actual
macOS listener via the designated local Lima host address. Both physical clients
close; reconnection retains one durable create receipt. Child-only Rust setup is
explicitly invoked by the runner and is not counted as a standalone default pass.

Credentials, private CA/key files, fixture state and raw logs remain ignored.
The guest build has no network; the guest rehearsal uses only the owned local
server. Default HTTP control remains loopback. This is an isolated guest topology,
not a named remote/provider rehearsal, distinct-host decoded media qualification
or a native x86_64 performance result. Publication is separate.
