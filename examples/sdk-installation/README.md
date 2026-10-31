# Installed SDK examples

Copy this directory outside the repository. These examples import ordinary
installed packages and resolve the matching native `connect` from PATH. They
read current capabilities and join their original helper; they send no messages.
Keep endpoint configuration and signing files private.

Use the native runtime candidate matching your platform and SDK version. Put its
`bin` directory first on PATH. A local synthetic installation can be prepared
with the packaged CLI (choose unused loopback ports):

```sh
connect init --directory /PRIVATE/NEW/installation --bind 127.0.0.1:7443 --http-bind 127.0.0.1:8080
python3 make-config.py /PRIVATE/NEW/installation
connect serve --config /PRIVATE/NEW/installation/server.toml
```

The initialization directory must be new. `make-config.py` refuses to overwrite
an existing SDK configuration and creates the new file with mode 0600 beside its
relative certificate/signing assets. This local example uses the initialized
operator; enroll a scoped identity for an application deployment.

In another terminal, install the Node archive and its pinned dependency archives
from the candidate, then run the example with Node 22 or later:

```sh
cd node
npm install --offline --ignore-scripts /CANDIDATE/node/dependencies/*.tgz /CANDIDATE/node/rudeless-connect-0.1.0.tgz
node inspect.mjs /PRIVATE/NEW/installation/operator.sdk.toml
```

For Python, create a fresh venv and install the candidate wheel without an index:

```sh
cd python
python3 -m venv .venv
.venv/bin/python -m pip install --no-index /CANDIDATE/python/rudeless_connect-0.1.0-py3-none-any.whl
.venv/bin/python -I inspect_candidate.py /PRIVATE/NEW/installation/operator.sdk.toml
```

Each successful invocation prints only the protocol profile and feature count.
An error must be investigated; these examples do not reconnect or retry a
mutation. The standalone Rust source bundle supplies its own offline Cargo
configuration and public journey example in `examples/rust-sdk-journey`.

For the Rust installation example, extract the standalone Rust source bundle and
run from its root so Cargo finds the supplied offline directory source:

```sh
cargo run --offline --manifest-path examples/sdk-installation/rust/Cargo.toml -- /PRIVATE/NEW/installation/operator.sdk.toml
```

It connects through the installed `connect-client` crate directly and closes the
original signed peer. No native helper executable is used by the Rust facade.

G6-069's complete matching native/package installation qualification is still in
progress. G6-070 owns the complete language/version/platform/facade/media matrix.
