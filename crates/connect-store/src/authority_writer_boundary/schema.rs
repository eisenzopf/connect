//! Persistent per-row enforcement, compiled from the audited fact dependencies.
//! Every current table has a registered-Unit guard. Authority-bearing rows also
//! require exact old/new and unique-collision footprints. Unknown schema fails.
use super::*;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Map {
    tables: Vec<Table>,
    all_tables: BTreeMap<String, Columns>,
}
#[derive(Deserialize)]
struct Columns {
    columns: Vec<String>,
}
#[derive(Deserialize)]
struct Table {
    table: String,
    scope_queries_for_each_row: Vec<ScopeQuery>,
    must_resolve_base_scope: bool,
}
#[derive(Deserialize)]
struct ScopeQuery {
    kind: String,
    id_sql: String,
    role: String,
}

fn identifier(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(Error::storage());
    }
    Ok(format!("\"{value}\""))
}
fn guard(kind: &str, id: &str) -> String {
    format!("{FUNCTION}('{kind}',{id})")
}
fn footprint(table: &Table, row: &str, skip: &str) -> Result<String> {
    let mut sql = String::new();
    if table.table == "installation" {
        return Ok(format!(
            "SELECT {};",
            guard("exclusive-installation", "NULL")
        ));
    }
    let bases = table
        .scope_queries_for_each_row
        .iter()
        .filter(|q| q.role == "base" || q.role == "non-conversation-owner-fallback")
        .map(|q| q.id_sql.replace("{row}", row))
        .collect::<Vec<_>>();
    if table.must_resolve_base_scope {
        if bases.is_empty() {
            return Err(Error::storage());
        }
        sql.push_str(&format!(
            "SELECT CASE WHEN NOT ({skip}) AND NOT EXISTS({}) THEN {} ELSE 1 END;",
            bases.join(" UNION ALL "),
            guard("unresolved-row", "NULL")
        ));
    }
    for query in &table.scope_queries_for_each_row {
        let kind = match query.kind.as_str() {
            "Identity" => "identity",
            "Conversation" => "conversation",
            "Record" => "record",
            _ => return Err(Error::storage()),
        };
        let ids = query.id_sql.replace("{row}", row);
        // The aggregate forces evaluation for every required delegated subject,
        // not merely the first row of a scalar subquery.
        sql.push_str(&format!(
            "SELECT COALESCE(sum({}),0) FROM ({ids}) WHERE NOT ({skip});",
            guard(kind, "id")
        ));
    }
    Ok(sql)
}
fn collision_predicate(connection: &Connection, table: &str) -> Result<String> {
    let quoted = identifier(table)?;
    let mut keys = Vec::<Vec<String>>::new();
    let mut primary = connection
        .prepare(&format!("PRAGMA table_info({quoted})"))
        .map_err(|_| Error::storage())?
        .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    primary.retain(|(order, _)| *order > 0);
    primary.sort_by_key(|(order, _)| *order);
    if !primary.is_empty() {
        keys.push(primary.into_iter().map(|(_, name)| name).collect());
    }
    let indexes = connection
        .prepare(&format!("PRAGMA index_list({quoted})"))
        .map_err(|_| Error::storage())?
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut predicates = Vec::new();
    for (index, unique) in indexes {
        if !unique {
            continue;
        }
        let columns = connection
            .prepare(&format!("PRAGMA index_info({})", identifier(&index)?))
            .map_err(|_| Error::storage())?
            .query_map([], |r| r.get::<_, Option<String>>(2))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        if columns.iter().all(Option::is_some) {
            keys.push(columns.into_iter().flatten().collect());
        } else if table == "endpoints" && index == "endpoints_address_claim" {
            // Ignore the partial-index WHERE conservatively: every possible
            // unique replacement is covered, including an inactive old row.
            predicates.push("existing.tenant_id=NEW.tenant_id AND existing.transport=NEW.transport AND lower(existing.address)=lower(NEW.address)".to_owned());
        } else {
            return Err(Error::storage());
        }
    }
    for key in keys {
        if key.is_empty() {
            return Err(Error::storage());
        }
        let parts = key
            .iter()
            .map(|column| {
                let column = identifier(column)?;
                Ok(format!("existing.{column}=NEW.{column}"))
            })
            .collect::<Result<Vec<_>>>()?;
        predicates.push(parts.join(" AND "));
    }
    predicates.sort();
    predicates.dedup();
    Ok(if predicates.is_empty() {
        "0".into()
    } else {
        predicates
            .into_iter()
            .map(|p| format!("({p})"))
            .collect::<Vec<_>>()
            .join(" OR ")
    })
}
fn same_original_key(connection: &Connection, table: &str) -> Result<String> {
    let columns = connection
        .prepare(&format!("PRAGMA table_info({})", identifier(table)?))
        .map_err(|_| Error::storage())?
        .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let keys = columns
        .into_iter()
        .filter(|(order, _)| *order > 0)
        .map(|(_, column)| {
            let column = identifier(&column)?;
            Ok(format!("existing.{column} IS OLD.{column}"))
        })
        .collect::<Result<Vec<_>>>()?;
    if keys.is_empty() {
        return Err(Error::storage());
    }
    Ok(keys.join(" AND "))
}
fn initial_slot(record: &str) -> String {
    format!("EXISTS(SELECT 1 FROM operations birth_op JOIN connections birth_conn ON birth_conn.id=birth_op.connection_id WHERE birth_op.record_id={record} AND birth_op.command_type='connection.offer' AND birth_conn.state='preparing' AND birth_conn.generation=1 AND birth_conn.revision=0 AND birth_conn.observed_runtime_id IS NULL AND birth_conn.readiness_json IS NULL AND NOT EXISTS(SELECT 1 FROM native_connection_lifetimes prior WHERE prior.connection_id=birth_conn.id) AND NOT EXISTS(SELECT 1 FROM telephone_connection_lifetimes prior WHERE prior.connection_id=birth_conn.id))")
}
fn dormant(table: &str, action: &str, collisions: &str) -> String {
    if action == "insert" {
        let no_collision =
            format!("NOT EXISTS(SELECT 1 FROM \"{table}\" existing WHERE {collisions})");
        let condition = match table {
            // No earlier positive grant can name a nonexistent unique namespace
            // or operation. This creates no reusable new-ID permit after commit.
            "identities" | "conversations" | "operations" => "1".to_owned(),
            "connections" => "NEW.state='preparing' AND NEW.generation=1 AND NEW.revision=0 AND NEW.observed_runtime_id IS NULL AND NEW.readiness_json IS NULL".into(),
            "connection_offers" => "EXISTS(SELECT 1 FROM connections c WHERE c.id=NEW.connection_id AND c.state='preparing' AND c.generation=1 AND c.revision=0 AND c.observed_runtime_id IS NULL AND c.readiness_json IS NULL)".into(),
            "native_offer_operations" => format!("NEW.effect_started=0 AND {}", initial_slot("NEW.operation_record_id")),
            "operation_members" | "operation_endpoints" | "operation_effects" => format!("{} AND EXISTS(SELECT 1 FROM native_offer_operations n WHERE n.operation_record_id=NEW.operation_record_id AND n.effect_started=0)", initial_slot("NEW.operation_record_id")),
            // The receipt and its unique original operation are created in one
            // SQL transaction before any native invocation can be constructed.
            "operation_control_receipts" => "NOT EXISTS(SELECT 1 FROM native_offer_operations n WHERE n.preparation_record_id=NEW.operation_record_id)".into(),
            _ => "0".into(),
        };
        return format!("({no_collision}) AND ({condition})");
    }
    if action == "update" && table == "sessions" {
        // Actual flow observations change the UI phase/revision, not native IO
        // permissions. All native public/private/continuation facts accept the
        // same three live states. Exported metadata is the saved journal prefix.
        // Every owner, audience, codec/topology or terminal mutation stays Write.
        let same = [
            "id",
            "conversation_id",
            "initiator_id",
            "participant_id",
            "media",
            "created_at",
            "ended_reason",
            "topology_revision",
            "audience_mode",
            "terminal_reason_code",
        ]
        .iter()
        .map(|column| format!("OLD.{column} IS NEW.{column}"))
        .collect::<Vec<_>>()
        .join(" AND ");
        return format!("({same}) AND OLD.state IN ('accepted','ready','active') AND (NEW.state IS OLD.state OR (OLD.state IN ('accepted','ready') AND NEW.state='active')) AND NEW.state_revision IN (OLD.state_revision,OLD.state_revision+1)");
    }
    if action == "update" && table == "journal" {
        // Existing SQL insert triggers fill epochs/time/fact metadata. Such a
        // row is outside every saved export prefix; no new-ID permission is
        // retained across commit, and identity/sequence cannot be rewritten.
        return "OLD.sequence IS NEW.sequence AND OLD.id IS NEW.id AND OLD.conversation_id IS NEW.conversation_id AND OLD.sequence>(SELECT COALESCE(MAX(watermark),0) FROM context_deliveries)".into();
    }
    if action == "update" && table == "operations" {
        let immutable = [
            "record_id",
            "tenant_id",
            "owner_id",
            "operation_id",
            "digest",
            "command_type",
            "conversation_id",
            "session_id",
            "connection_id",
            "expected_revision",
            "intent_json",
            "result_json",
            "problem_json",
            "tombstoned",
            "created_at",
            "expires_at",
            "ledger_version",
            "worker_cleanup",
            "cancel_requested",
        ]
        .iter()
        .map(|column| format!("OLD.{column} IS NEW.{column}"))
        .collect::<Vec<_>>()
        .join(" AND ");
        // The original Accepted offer has never satisfied current(): that
        // requires Preparing plus the exact submitting effect/worker lease.
        // Permit only that one birth, never a later Preparing mutation.
        return format!("({immutable}) AND OLD.command_type='connection.offer' AND OLD.state='accepted' AND NEW.cancel_requested=0 AND NEW.worker_generation=1 AND OLD.worker_generation IN (0,1) AND ((NEW.state='accepted' AND NEW.revision=OLD.revision) OR (NEW.state='preparing' AND NEW.revision=OLD.revision+1)) AND NEW.lease_expires_at IS NEW.expires_at AND {} AND EXISTS(SELECT 1 FROM native_offer_operations n JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=n.operation_record_id WHERE n.operation_record_id=NEW.record_id AND n.effect_started=1 AND e.state='submitting' AND NEW.lease_owner IS n.attempt_id AND NEW.worker_runtime IS n.attempt_runtime)", initial_slot("NEW.record_id"));
    }
    if action == "update" && table == "native_offer_operations" {
        // Exactly the initial one-way effect birth. Every OLD=1 mutation, even
        // during uncertain preparation, remains under the exact row writer.
        return format!("OLD.effect_started=0 AND NEW.effect_started=1 AND OLD.operation_record_id IS NEW.operation_record_id AND OLD.preparation_record_id IS NEW.preparation_record_id AND OLD.credential_id IS NEW.credential_id AND OLD.key_id IS NEW.key_id AND OLD.device_id IS NEW.device_id AND OLD.endpoint_id IS NEW.endpoint_id AND OLD.participant_id IS NEW.participant_id AND OLD.membership_epoch IS NEW.membership_epoch AND OLD.generation IS NEW.generation AND OLD.resources_json IS NEW.resources_json AND OLD.source_expires_at IS NEW.source_expires_at AND OLD.terminal_reserve IS NEW.terminal_reserve AND OLD.attempt_id IS NULL AND OLD.effect_id IS NULL AND OLD.attempt_runtime IS NULL AND NEW.attempt_id IS NOT NULL AND NEW.effect_id IS NOT NULL AND NEW.attempt_runtime IS NOT NULL AND {} AND EXISTS(SELECT 1 FROM operation_effects e WHERE e.id=NEW.effect_id AND e.operation_record_id=NEW.operation_record_id AND e.state='submitting')", initial_slot("NEW.operation_record_id"));
    }
    "0".into()
}
fn definitions(connection: &Connection) -> Result<Vec<(String, String, String)>> {
    let map: Map =
        serde_json::from_str(include_str!("footprints.json")).map_err(|_| Error::storage())?;
    let actual = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").map_err(|_| Error::storage())?
        .query_map([], |r| r.get::<_, String>(0)).map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(|_| Error::storage())?;
    if actual != map.all_tables.keys().cloned().collect::<Vec<_>>() {
        return Err(Error::storage());
    }
    let mut definitions = Vec::new();
    for table in actual {
        let quoted = identifier(&table)?;
        let columns = connection
            .prepare(&format!("PRAGMA table_info({quoted})"))
            .map_err(|_| Error::storage())?
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<BTreeSet<_>>>()
            .map_err(|_| Error::storage())?;
        if columns != map.all_tables[&table].columns.iter().cloned().collect() {
            return Err(Error::storage());
        }
        let policy = map.tables.iter().find(|policy| policy.table == table);
        let collisions = if policy.is_some() {
            collision_predicate(connection, &table)?
        } else {
            "0".into()
        };
        for action in ["insert", "update", "delete"] {
            let name = format!("connect_writer_{table}_{action}");
            let mut body = format!("SELECT {};", guard("bookkeeping", "NULL"));
            if let Some(policy) = policy {
                let skip = if table == "journal" && action == "insert" {
                    "1".into()
                } else {
                    dormant(&table, action, &collisions)
                };
                for row in if action == "update" {
                    vec!["OLD", "NEW"]
                } else if action == "insert" {
                    vec!["NEW"]
                } else {
                    vec!["OLD"]
                } {
                    body.push_str(&footprint(policy, row, &skip)?);
                }
                if action == "insert" || action == "update" {
                    let collision_rows = if action == "update" {
                        format!(
                            "({collisions}) AND NOT ({})",
                            same_original_key(connection, &table)?
                        )
                    } else {
                        collisions.clone()
                    };
                    // Correlated scalar aggregates visit every old collision's
                    // entire dependency footprint, before REPLACE can erase it.
                    let old = footprint(policy, "existing", "0")?;
                    for statement in old.split(';').filter(|s| !s.is_empty()) {
                        body.push_str(&format!(
                            "SELECT ({statement}) FROM {quoted} existing WHERE {collision_rows};"
                        ));
                    }
                }
            }
            let sql = format!(
                "CREATE TRIGGER {} BEFORE {} ON {quoted} BEGIN {body} END",
                identifier(&name)?,
                action.to_ascii_uppercase()
            );
            definitions.push((name, table.clone(), sql));
        }
        if table == "journal" {
            let name = "connect_writer_journal_append".to_owned();
            // BEFORE enforces registered original SQL custody and every unique
            // collision. AFTER validates the actual assigned sequence: implicit
            // NEW.rowid before insertion is not a reliable append witness.
            let safe = "NEW.sequence>(SELECT COALESCE(MAX(sequence),0) FROM journal WHERE id!=NEW.id) AND NEW.sequence>(SELECT COALESCE(MAX(watermark),0) FROM context_deliveries)";
            let body = format!(
                "SELECT CASE WHEN {safe} THEN {} ELSE {} END;",
                guard("bookkeeping", "NULL"),
                guard("conversation", "NEW.conversation_id")
            );
            definitions.push((
                name.clone(),
                table.clone(),
                format!(
                    "CREATE TRIGGER {} AFTER INSERT ON {quoted} BEGIN {body} END",
                    identifier(&name)?
                ),
            ));
        }
    }
    Ok(definitions)
}
pub(crate) fn install(connection: &Connection) -> Result<()> {
    for (_, _, sql) in definitions(connection)? {
        connection
            .execute_batch(&sql)
            .map_err(|_| Error::storage())?;
    }
    validate(connection)
}
pub(crate) fn validate(connection: &Connection) -> Result<()> {
    for (name, table, expected) in definitions(connection)? {
        let actual: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1 AND tbl_name=?2",
                [&name, &table],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if actual.as_deref() != Some(expected.as_str()) {
            return Err(Error::storage());
        }
    }
    Ok(())
}
