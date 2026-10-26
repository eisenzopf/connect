# Connector contribution kit v2

`scripts/export-connector-kit.py` creates a fresh source snapshot for independent
message and live-media contributions. The output is component development
material. Runtime/SDK candidate packages, clean-cache/platform qualification,
stock application journeys, real provider/model rehearsals and publication remain
separately assigned Gate 6 work.

Included packages are `connect-connector-api`, `connect-media-conformance`, the
message/PCM template, independent Prism fixture and actual outbound-PCMU fixture.
The media graph includes only its selected public prepared rvoip dependencies.
Connect Store, service, server, application databases, credentials, raw logs,
private state, Git history and source symlinks are excluded. No `.deps` tree,
private old checkout or historical executable is a build prerequisite.

Before export, the repository preparation verifies the original source lock and
patch digests. The exporter copies exact Rust sources from that preparation and
records their hashes. `connector-kit-source.json` records original upstream
identities, the original source-lock digest, complete kit file hashes and
original/generated consumer manifest hashes. Every manifest normalization has an
explicit patch and preserved original under `provenance/manifests/`.

Generated manifests flatten workspace package/dependency/lint inheritance and
relocate local dependency paths inside the kit. Normal dependency and feature
declarations remain; all registry package name/version/source sets must match
the original fixture graphs. Engine dependency manifests exclude upstream
engine-local development dependencies and targets. Those require other engine
products and are not part of this consumer kit's qualification. Their original
manifests remain available; do not claim the kit executes their upstream suites.
No Rust source is rewritten, and the original repository `.deps` is untouched.

The kit includes the repository Rust toolchain pin; it never changes the global
Rust default. Every fixture keeps its own independent Cargo workspace and lock file. The kit
verifier rejects file inventory changes, source symlinks, missing/inconsistent
locked metadata or any local Cargo dependency outside the kit. Registry caches
remain external prerequisites, explicitly selected build targets stay private,
and offline builds do not imply a bundled registry cache or clean-cache proof.

From the Connect checkout:

```sh
python3 scripts/export-connector-kit.py --output /absolute/new/kit
```

From the exported kit, follow its README and use fresh private report paths:

```sh
python3 scripts/export-connector-kit.py --output "$PWD" --verify
python3 scripts/check-connector-contract.py --manifest-path examples/connector-template/Cargo.toml --target-dir /absolute/private/target --output /absolute/private/message-report.json
python3 scripts/check-connector-contract.py --manifest-path examples/connector-contribution/Cargo.toml --target-dir /absolute/private/target --output /absolute/private/prism-report.json
python3 scripts/check-media-connector-contract.py --target-dir /absolute/private/target --output /absolute/private/media-report.json
python3 scripts/check-media-connector-contract.py --target-dir /absolute/private/target --negative-control --output /absolute/private/media-negative.json
```

The message reports execute `connect-connector-fixture-v2`. The actual media
report executes `connect-live-media-fixture-v1`. Negative custody is an expected
refusal with `fixture_passed:false`, never a passing transport. The factory and
instrumentation remain fixture controls, with no production admission authority.
Stock canonical host routing, private audiences, uncertainty, reconnect, cancel
and teardown require their separate current application-level evidence.
