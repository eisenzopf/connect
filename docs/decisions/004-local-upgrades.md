# 004 — explicit local upgrades and snapshot rollback

`serve` never creates or migrates a database. It requires schema 2 and refuses
schema 1 or an unknown newer version before reporting ready. One installation
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
compares all schema-1 logical state with that snapshot inside an immediate
transaction. Stale or foreign backups fail. Schema 1→2 runs atomically; a
failed migration leaves the prior version. No automatic down migration exists.

Rollback uses the prior milestone binary plus its matching pre-upgrade
snapshot, restored to a new directory. Post-backup work is not present in that
snapshot and must be recovered separately. The old binary is not permitted to
read schema 2. The current local process drill preserves the actual schema-1
milestone binary under ignored `.local/milestones/m1/`, checks its recorded
SHA-256, and proves both refusal and rollback. M5 must retain this artifact and
its manifest with the local qualification materials; it is not a published
release or a claimed reproducible source build of that earlier binary.

Normal binaries contain no test failpoint controls. An explicitly featured
`test-support` binary exits abruptly only for a named synthetic request and
stage. Test state is disposable; real review installations are never faulted.
