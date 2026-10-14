# Public recovery prerequisites

Gate 5 recovery uses disposable local state and public source pins from
`dependencies/recovery-baselines.json` in the matching source archive.
Neither an owner database nor a private milestone binary is required.

```sh
python3 scripts/build-recovery-baseline.py --schema 1 --public-source
python3 scripts/build-recovery-baseline.py --schema 16 --public-source
python3 scripts/check-gate5-recovery.py \
  --binary /absolute/path/to/connect \
  --schema-one-binary /absolute/path/to/schema1/connect \
  --gate4-binary /absolute/path/to/schema16/connect
```

The builder fetches the exact public commit into ignored `.local/` scratch,
prepares that commit's pinned upstream dependencies and recorded patches, and
builds with its original Cargo lock. Its adjacent `manifest.json` identifies
source repository/revision/tree, binary hash, Cargo lock, upstream graph, Rust
version and any fixture patch. A source archive needs the builder, catalog and
patch; it does not need local Git history. Network access to public Git/Cargo
sources and the documented local toolchain/native libraries remain prerequisites.

Public history first introduces implementation at schema 2 in
[`3aa6077`](https://github.com/eisenzopf/connect/commit/3aa6077bee3ea05cea0aca550e9469df53c5ed87).
The original private schema 1 milestone has no recorded source revision. The
reproducible schema 1 prerequisite is therefore an **identity-only recovery
fixture**, built from that public source with the digest-pinned
schema 1 initialization patch (`patches/recovery/schema1-identity-fixture.patch` in the matching source archive).
The patch sets the supported schema to 1 and omits conversation DDL during
initialization. It preserves the original identity/authentication/TLS runtime.
It is not an unmodified historical release and does not qualify messaging on
schema 1. Its purpose is actual executable initialization, health, upgrade
refusal and matching-snapshot rollback against the schema 1 contract.

Schema 16 uses the unmodified public, installed Gate 4 candidate source
[`32f4fad`](https://github.com/eisenzopf/connect/commit/32f4fadeaf45b1d59432a552b30522a94d388eb0),
identified in [Gate 4 package evidence](evidence/gate-4/package-candidate.json).
The separate Gate 5 regression baseline at `993771a` retains its own source and
failure history; these identities are never substituted for one another.

The Gate 5 driver preserves conversations, message receipts, managed artifact
bytes, tasks and policy rows across the schema 16 upgrade, tests transaction
rollback after DDL failure, and uses a verified matching backup with the old
runtime. It checks current task and pairing authority after restore at the explicitly selected candidate’s current schema,
partial destinations, checksum corruption and an interrupted backup before its
completion marker. First-run human configuration, setup metadata and protected
secret references accompany a backup. Keychain values are never exported: vault
recovery requires the original protected entry, or the explicitly authenticated
original-value import described in [setup](SETUP.md).

Existing Gate 4 builder/checker defaults and immutable archives remain unchanged.
The builder still defaults to schema 6 and supports the retained schema 10 pin.
Check results are recorded separately; building prerequisites does not qualify a
migration or close Gate 5.

Installed qualification selects every artifact explicitly. The SDK root must
contain `connect_sdk/`; it is selected before any SDK import and never replaced
by checkout modules. Flat qualification archives can supply distinct manifest
files for each binary:

```sh
python3 scripts/check-gate5-recovery.py \
  --binary /candidate/bin/connect \
  --schema-one-binary /qualification/connect-schema1 \
  --schema-one-manifest /qualification/manifest.json \
  --gate4-binary /qualification/connect-schema16 \
  --gate4-manifest /qualification/schema16-manifest.json \
  --catalog /qualification/recovery-baselines.json \
  --python-sdk /candidate/sdk/python \
  --work-root .local/installed-recovery \
  --output .local/installed-recovery-result.json
```

Results contain `status`, individual phase outcomes/timing, metadata-only
observations, the exact candidate schema discovered through fresh public initialization and independently checked against its database version, binary and historical provenance hashes, and hashes of the
selected catalog, Python SDK and checker source. A failed run retains its private
traceback and state. The output file must be fresh; an earlier result is never
overwritten. New schema 1/16/current-schema proof supplements the retained schema 6/10
executable regressions; it does not silently replace their historical coverage.

The October 13 source check built both public prerequisites and passed all six
recovery phases against the fresh normal binary, preserving 38 historical tables,
51 rows and 28,672 artifact bytes. Provenance, phase timings and the actual killed
backup/restore checks are recorded in
[migration recovery evidence](evidence/gate-5/migration-recovery.json).
The unchanged Gate 4 baseline and installed candidate checks remain separate;
these source results do not close Gate 5.
