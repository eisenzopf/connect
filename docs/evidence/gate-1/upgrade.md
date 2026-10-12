# M5 — packaged local upgrade and rollback

`check-package.py` invoked `check-recovery.py` with the installed candidate as
`CONNECT_BINARY` and the separately archived historical binary as
`CONNECT_PREVIOUS_BINARY`. Exit 0.

The historical schema-1 binary checksum is
`f22ec499a9a6dcf0d899a942a6ccd768e0cffa0f88ff86502070cec82c67e225`.
It initialized fresh legacy state; no private old snapshot or credentials were
copied. The current candidate captured a consistent backup and migrated a
stopped copy 1→2. The old binary refused schema 2. Restoring the matching backup
into a new directory allowed the actual old binary to run again.

An intentionally conflicting schema object made migration fail. The transaction
left user_version 1; a fresh restore recovered the prior compatible state.
Foreign/stale backups and unsupported newer schemas are covered by the store
suite. The original installation's post-backup accepted request remained intact.

Only the previous binary and its public provenance manifest enter the separate
qualification archive. The original `.local/milestones/m1/state/` remains
untouched. Historical source-build reproducibility is not claimed. Rollback
loses work accepted after its snapshot; no down migration is provided.
