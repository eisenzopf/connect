# 004 — explicit local upgrades and snapshot rollback

`serve` never creates or migrates a database. The active development code requires schema 6 and refuses
older or unknown schemas before reporting ready. One installation
lock, keyed to the canonical database path, prevents two current server
instances from writing the same installation even on different ports.

`backup --server-config PATH --config OPERATOR --output NEW_DIRECTORY` uses
SQLite's online backup API, including committed WAL contents. It copies the
current TLS pair and normalized server/operator/demo configs into a protected
directory and writes a checksum manifest last. The manifest is a completion /
integrity marker, not proof of authenticity against a malicious backup author.
Backups contain sensitive recovery material and must remain protected.
Client configs issued elsewhere and app-owned job state are separately retained
by their owners; this installation backup does not discover arbitrary files.

`restore --backup DIRECTORY --directory NEW_DIRECTORY --bind LOOPBACK` verifies
all allowlisted paths, file hashes, database integrity and logical snapshot
identity before creating a fresh destination. Existing destinations are never
overwritten. A cursor newer than the restored snapshot fails explicitly.

Stop the affected server before `migrate`. The command checks current operator
authority, holds the installation lock, verifies the backup manifest, and
compares all prior-schema logical state with that snapshot inside an immediate
transaction. Stale or foreign backups fail. Supported schema 1–5 →6 migrations run atomically; a
failed migration leaves the prior version. No automatic down migration exists.

Rollback uses the prior milestone binary plus its matching pre-upgrade
snapshot, restored to a new directory. Post-backup work is not present in that
snapshot and must be recovered separately. An older binary must not read a newer schema. The historical Gate 1
qualification below specifically covered schema 1→2; it is not evidence for
the new development schema 6. The current local process drill preserves the actual schema-1
milestone binary under ignored `.local/milestones/m1/`, checks its recorded
SHA-256, and proves both refusal and rollback. M5 must retain this artifact and
its manifest with the local qualification materials; it is not a published
release or a claimed reproducible source build of that earlier binary.

Normal binaries contain no test failpoint controls. An explicitly featured
`test-support` binary exits abruptly only for a named synthetic request and
stage. Test state is disposable; real review installations are never faulted.

Schema 5 adds explicit inbound voice bindings and call direction. Schema-4 voice
rows migrate as outbound; existing journal JSON remains readable with that
default. Snapshot digests include inbound bindings; migration tests retain and
verify the unchanged prior-schema backup.

Schema 6 adds a durable inbound discovery watermark and the credential that
authorized an end request. Earlier pending end requests have no attributable
credential and migrate to reconciliation-only state: the server reads provider
status without repeating the external end mutation. New end requests recheck
their actual requester's authority at dispatch. The schema-5 migration fixture
verifies these semantics, preserved bindings/calls and an unchanged backup.
