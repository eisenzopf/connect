# Local ARM64 Linux development and media testing

Use the dedicated native ARM64 Colima VM on the Mac Studio. This is a local
component profile with the complete native workload and unchanged quality and
deadline checks. The owner selected ARM64 as the active Linux target on November2; the32
required cells now use MacARM64 and UbuntuARM64, with x86 deferred. Do not dispatch media qualification to GitHub-hosted runners.

The existing profile was created with:

```sh
colima start connect-gate6 --arch aarch64 --cpus 6 --memory 12 --disk 60 \
  --vm-type vz --activate=false --mount none --binfmt=false
docker --context colima-connect-gate6 build --platform linux/arm64 \
  -t connect-gate6-local-arm64:1 scripts/fixtures/local-linux
```

Keep the current Docker context and existing database VM intact. Use the explicit
context for every command. Copy a clean source checkout into an owned container's
guest storage, including its Git source identity; exclude host `.local`, account
credentials and browser profiles. Prepare dependencies inside that checkout with
`bash scripts/prepare-dependencies.sh` before building. Record image ID, package
inventory, architecture, source hashes and executable hashes with each run.

As the image's ordinary `connect` user, run:

```sh
python3 scripts/check-transport-fixtures.py --arm64-component \
  --media-workload --without-host-capture --signing-algorithm EdDSA
```

This starts fresh owned synthetic state. Never reuse an occupied fixture
directory or resubmit an uncertain original operation. The explicit capture skip
is reported and cannot establish packet-capture acceptance. Keep raw logs,
SQLite/WAL, media and credentials in ignored `.local`. Retain failed outcomes.
Builds and tests use guest disk storage, without a mounted host checkout. Run
timing observations after other owned compilation has finished. x86 qualification is deferred by the owner; the diagnostic component still cannot
qualify any complete transport cell.

For the reference app, package the native production runtime with
`scripts/package-sdk-runtime.py --target aarch64-unknown-linux-gnu` and the
required owned `--output`/`--evidence` directories. Produce the normal Node
installation proof with `sdk/tests/check_node_installation.py`, then install both
archives through `scripts/install-reference-app-components.py`. The runtime
producer verifies an AArch64 ELF executable and records ARM component scope;
the ordinary offline installer retains all archive and source checks.

Run `scripts/check-reference-app-installed.mjs` against that installation with
`--arm64-component`. The owned Playwright1.64.1 Chromium1248 target is frozen to
156.0.8078.4 in `profile.json` before its first execution. Browser installation
and prerequisites must complete before measuring media. All 37 app checks and
their audio/video deadlines remain unchanged. ARM reports identify the actual
architecture. Every transport cell still needs its complete independent evidence.

On a cold guest, fetch the complete locked target dependency metadata before
packaging: `cargo fetch --locked --target aarch64-unknown-linux-gnu`. A production
build alone may not populate workspace development dependencies needed by the
packager's later offline metadata check. Keep the ordinary consumer installation
offline. Give `check_node_installation.py` an owned temporary directory outside
the source checkout: its empty-cache consumer intentionally refuses an in-repo
`TMPDIR`. Use a new proof-directory leaf on each attempt; its Node archive lives
under `dist/sdk/typescript/<proof-directory-leaf>/`.

All active Gate6 Linux runner requirements now use Ubuntu24.04 ARM64, native
AArch64 Rust and ELF machine183. Package names use `ubuntu-24.04-arm64` and
`aarch64-unknown-linux-gnu`; positive fixture definitions follow those identities.
Mac native cases remain arm64. Old measured x86 results keep their original
scope; changing a runner requirement does not rerun or qualify it.

Run the CLI fixtures directly in the local VM. Workflow definitions retain only
manual/reusable entrypoints and require explicitly named local self-hosted ARM
labels; no workflow is dispatched and no runner registration is needed for the
local procedure. Automatic push/PR test triggers are removed. GitHub is used for
source synchronization and pushes.

For a browser image, use a new guest-storage image context containing only the
verified public Chromium1248/CFT156.0.8078.4 ARM distribution and the locked
Playwright package inputs. `scripts/prepare-local-browser-image.py --browser
/absolute/path/to/chromium-1248/chrome-linux-arm64/chrome --output
.local/gate6-browser-media/image-context` creates that context, verifies native
ELF183 and the installed browser version, and rejects escaping symlinks. Build
`scripts/video/Dockerfile.browser-media` with `--platform linux/arm64` and
`--pull=false --build-arg BUILD_BASE_IMAGE=<digest-verified-local-ARM-build-image-tag>`. Record the
resulting immutable image ID and package inventory before the test. Never commit
or snapshot a running fixture's private repository/state into an image.

Select the local immutable browser image through
`CONNECT_GATE6_ARM_BROWSER_IMAGE=sha256:<recorded-image-id>`. Native/telephone
network producers inspect this existing ARM image and never pull the historical
x86 image. Browser media, direct/relay bridge, expiry and impairment runners
verify native ARM kernel/toolchain and browser image/asset identities. Current
source/binary guards, original read/programme/cleanup deadlines and media limits
remain required.

Dockerfile `FROM` resolves image references rather than a bare local image ID.
Tag the existing ARM build image with its full verified image ID in the tag,
verify that tag resolves to the original ID before and after the build, and pass
that local tag as `BUILD_BASE_IMAGE` with `--pull=false`. Record both the image ID
and tag. The G054 TURN target is the ARM64 child of coturn4.18.0:
`coturn/coturn@sha256:0a583c17085e1629e3a4ebe6c505aba82cbdd1733557f6932857e3327f5f3068`.
The older `59af...` digest is its amd64 sibling and must never run in the active
ARM profile. Inspect actual image architecture and the unchanged4.18.0 version
before the selected test.
