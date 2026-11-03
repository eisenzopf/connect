# Local ARM64 Linux development and media testing

Use the dedicated native ARM64 Colima VM on the Mac Studio. This is a local
component profile with the complete native workload and unchanged quality and
deadline checks. It does not add an ARM entry to the 32 required acceptance
cells. Do not dispatch media qualification to GitHub-hosted runners.

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
timing observations after other owned compilation has finished. ARM results
remain separate from the required native x86_64 qualification.

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
architecture and cannot inherit an x86_64 or macOS browser result.
