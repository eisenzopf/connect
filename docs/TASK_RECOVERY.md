# Local Gate 4 recovery qualification

[G4-032](connect-gate-4-implementation.html#G4-032) requires historical upgrade,
verified backup, restore and rollback. Its [local evidence](evidence/gate-4/historical-recovery.json)
is separate from final installed-candidate and integrated Gate 4 acceptance.
All checks below use fresh local state. They do not migrate an existing server
or contact a provider account.

## Reproduce the executable checks

Prepare and build the current binary, then build the two pinned historical
baselines from a checkout containing their Git history:

```sh
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-server --bin connect
python3 scripts/build-recovery-baseline.py --schema 6
python3 scripts/build-recovery-baseline.py --schema 10
python3 scripts/check-gate4-recovery.py \
  --previous-binary .local/recovery-baselines/99243f3962cf25e780bbd176e56a4f27c752e121/connect
```

Schema 6 is pinned to `8bf5bd31a519a5fa84ef8f80e4da3b58374c6de5` and the
pre-Gate-4 schema-10 candidate to `99243f3962cf25e780bbd176e56a4f27c752e121`.
The baseline builder prepares each historical revision's own dependencies in
an isolated directory. Its manifest records source, tree and executable hash.
Rebuilt executable bytes can differ from an archived build; both must identify
the pinned source and pass the actual executable checks.

If retaining the originally qualified pre-Gate-4 candidate, pass its executable
with `--previous-binary`. The adjacent package manifest must identify schema 10,
the pinned revision and the matching `bin/connect` hash. With no override, the
checker uses `.local/gate-4-baseline/connect`. `--schema-six-binary` and `--binary`
select the other executables explicitly. A missing executable or wrong digest
fails; nothing is fetched or built implicitly by the checker.

The checker has a 600-second budget. Private logs, source hashes, phase results,
failure tracebacks and disposable installations remain under
`.local/gate4-recovery/<timestamp>/`. Failed attempts are retained. It compares
all original table columns and rows, checks SQLite integrity and foreign keys,
and verifies artifact bytes and public operation receipts.

## Recovery behavior covered

- The actual schema-10 binary creates a conversation, message receipt and
  28 KiB draft artifact. Upgrade preserves original data; the old executable
  refuses the new schema. Its matching pre-upgrade snapshot restores and runs
  with that old executable.
- Synthetic SMS authority, enrollment and STOP rows survive the migration.
  These are disposable database fixtures, not consent or live SMS evidence.
- A deliberate table conflict uses a verified backup of the same failing input.
  Migration reaches the DDL failure and rolls back without partial schema or
  row changes. The earlier good snapshot remains recoverable.
- A current-schema backup preserves a waiting task, journal cursor and original
  question receipt. After restoration into a new directory and port, a bound
  reply yields one follow-up derived from its authorized recalled source.
  Duplicate requests retain their original receipts. Cancellation then denies
  replay without changing the restored database.
- A real backup process is killed after its SQLite copy but before its final
  manifest, using a disposable FIFO to hold the copy boundary. Restore refuses
  the incomplete snapshot and does not create an installation. A fresh backup
  restores successfully; the interrupted output and original remain intact.
- The existing actual schema-6 upgrade/refusal/rollback regression runs under
  the same deadline and owned-process cleanup.

The separate uncertainty regression uses a loopback provider that accepts a
request but withholds its acknowledgement:

```sh
cargo test --locked -p connect-server --features test-support \
  --test gate4_task_uncertainty -- --test-threads=1
```

It checks ordinary restart and verified snapshot restoration. The same uncertain
effect survives without another provider dispatch; cancelling the restored task
revokes its invocation grant. The mock's independent acceptance count must remain
one. This does not establish a real provider's reconciliation API or delivery.

## Operational boundary

Rollback means the old executable with its matching pre-upgrade snapshot in a
new directory. It never means starting an old executable against a new schema.
Keep the original installation until the restored one has been checked.

The server backup includes its database, referenced blobs, TLS material and
documented client/configuration files. External host installations, private
grant exports and application-owned runner databases are separate assets; retain
them under their documented protection and recovery rules. Restore does not
recreate model sessions, renew grants, replay uncertain remote work or resume a
host automatically. Use [task diagnostics](TASK_DIAGNOSTICS.md), the
[host guide](CODEX_TASK_HOST.md) and [task access](TASK_ACCESS.md) to inspect and
continue with current authority.

These checks do not qualify power-loss durability, the final installed release,
all integrated authority cases, or live host/provider journeys. The numbered
plan keeps those requirements open.
