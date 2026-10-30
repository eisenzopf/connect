//! Bounded recorded binding metadata. This is never a media authority grant.
use crate::{Error, Result};
use connect_protocol::{
    events::{ConnectionState, StreamState},
    native_gates::GateStream,
    native_streams::CodecBinding,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
const MAX_REV: u64 = 9_007_199_254_740_991;
const MAX_RECORDS: usize = 64;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recorded {
    id: String,
    medium: String,
    generation: u64,
    state: String,
    revision: u64,
}
fn invalid() -> Error {
    Error::new(
        "invalid-observation",
        "Invalid captured native stream metadata",
    )
}
fn bump(revision: u64) -> Result<u64> {
    revision
        .checked_add(1)
        .filter(|v| *v <= MAX_REV)
        .ok_or_else(invalid)
}
fn records(value: &Value) -> Result<Vec<Recorded>> {
    let Some(metadata) = value.get("stream_metadata") else {
        return Ok(vec![]);
    };
    let out: Vec<Recorded> = serde_json::from_value(metadata.clone()).map_err(|_| invalid())?;
    if out.len() > MAX_RECORDS || serde_json::to_vec(&out).map_err(|_| invalid())?.len() > 32 * 1024
    {
        return Err(invalid());
    }
    let counter = value["stream_metadata_counter"]
        .as_u64()
        .filter(|v| *v <= MAX_REV)
        .ok_or_else(invalid)?;
    let mut ids = BTreeSet::new();
    for v in &out {
        connect_protocol::transitions::id(&v.id)?;
        if !ids.insert(&v.id)
            || !matches!(v.medium.as_str(), "audio" | "video" | "screen" | "data")
            || !matches!(
                v.state.as_str(),
                "negotiated" | "active" | "paused" | "ended"
            )
            || v.generation == 0
            || v.generation > MAX_REV
            || v.revision == 0
            || v.revision > counter
        {
            return Err(invalid());
        }
    }
    Ok(out)
}
pub(crate) fn validate(bindings: &[CodecBinding], generation: u64) -> Result<()> {
    if bindings.len() > 32 || generation == 0 || generation > MAX_REV {
        return Err(invalid());
    }
    let mut ids = BTreeSet::new();
    for b in bindings {
        connect_protocol::transitions::id(&b.stream_id)?;
        if !ids.insert(&b.stream_id)
            || b.connection_generation != generation
            || b.stream_generation == 0
            || b.stream_generation > MAX_REV
            || !matches!(b.medium.as_str(), "audio" | "video" | "screen" | "data")
        {
            return Err(invalid());
        }
    }
    Ok(())
}
pub(crate) fn replace(
    value: &mut Value,
    generation: u64,
    expected: Option<u64>,
    revision: u64,
    bindings: &[CodecBinding],
) -> Result<()> {
    validate(bindings, generation)?;
    if revision == 0 || revision > MAX_REV {
        return Err(invalid());
    }
    // Independent monotonically increasing resource observations. Retiring a
    // bounded terminal summary must never reset a reused ID's version later.
    let counter = bump(value["stream_metadata_counter"].as_u64().unwrap_or(0))?;
    let previous: BTreeMap<_, _> = records(value)?
        .into_iter()
        .map(|v| (v.id.clone(), v))
        .collect();
    let current = expected.is_some()
        && value["stream_metadata_revision"].as_u64() == expected
        && value["stream_metadata_generation"] == generation;
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    for b in bindings {
        ids.insert(b.stream_id.clone());
        let old = previous.get(&b.stream_id);
        if old.is_some_and(|v| v.medium != b.medium) {
            return Err(invalid());
        }
        let state = old
            .filter(|v| current && v.state != "ended")
            .map(|v| v.state.clone())
            .unwrap_or_else(|| "negotiated".into());
        let resource_revision = match old {
            Some(v)
                if value["stream_metadata_generation"] == generation
                    && v.generation == b.stream_generation
                    && v.state == state =>
            {
                v.revision
            }
            Some(_) => counter,
            None => counter,
        };
        out.push(Recorded {
            id: b.stream_id.clone(),
            medium: b.medium.clone(),
            generation: b.stream_generation,
            state,
            revision: resource_revision,
        });
    }
    let mut ended = Vec::new();
    for (_, mut v) in previous {
        if !ids.contains(&v.id) {
            if v.state != "ended" {
                v.state = "ended".into();
                v.revision = counter;
            }
            ended.push(v);
        }
    }
    // Only terminal summaries may be evicted. Operation records/journal retain
    // their original identities; an old omitted Stream is explicitly unavailable.
    ended.sort_by(|a, b| b.revision.cmp(&a.revision).then_with(|| a.id.cmp(&b.id)));
    out.extend(ended.into_iter().take(MAX_RECORDS - out.len()));
    out.sort_by(|a, b| a.id.cmp(&b.id));
    value["stream_metadata_counter"] = json!(counter);
    value["stream_metadata"] = serde_json::to_value(out).map_err(|_| invalid())?;
    value["stream_metadata_revision"] = json!(revision);
    value["stream_metadata_generation"] = json!(generation);
    Ok(())
}
fn load(c: &Connection, connection: &str) -> Result<Value> {
    let raw: Option<String> = c
        .query_row(
            "SELECT projection_json FROM native_stream_snapshots WHERE connection_id=?1",
            [connection],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    raw.map(|v| serde_json::from_str(&v).map_err(|_| Error::storage()))
        .transpose()
        .map(|v| v.unwrap_or_else(|| json!({})))
}
pub(crate) fn save(
    c: &Connection,
    connection: &str,
    generation: u64,
    revision: u64,
    value: &Value,
) -> Result<()> {
    records(value)?;
    c.execute("INSERT INTO native_stream_snapshots(connection_id,generation,stream_revision,available,projection_json) VALUES(?1,?2,?3,1,?4) ON CONFLICT(connection_id) DO UPDATE SET generation=excluded.generation,stream_revision=excluded.stream_revision,available=1,projection_json=excluded.projection_json WHERE excluded.generation>native_stream_snapshots.generation OR (excluded.generation=native_stream_snapshots.generation AND excluded.stream_revision>=native_stream_snapshots.stream_revision)",params![connection,generation,revision,value.to_string()]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(crate) fn complete(
    c: &Connection,
    connection: &str,
    generation: u64,
    expected: u64,
    revision: u64,
    bindings: Option<&[CodecBinding]>,
) -> Result<()> {
    if let Some(bindings) = bindings {
        let mut value = load(c, connection)?;
        replace(&mut value, generation, Some(expected), revision, bindings)?;
        save(c, connection, generation, revision, &value)
    } else {
        // A known effect result does not establish a matching descriptor. Keep
        // the historical description, but make fresh live reads unavailable.
        c.execute("UPDATE native_stream_snapshots SET available=0 WHERE connection_id=?1 AND generation=?2 AND stream_revision<=?3",params![connection,generation,revision]).map_err(|_|Error::storage())?;
        Ok(())
    }
}
pub(crate) fn gates(
    c: &Connection,
    connection: &str,
    generation: u64,
    expected: u64,
    revision: u64,
    streams: &[GateStream],
) -> Result<()> {
    let available: bool = c
        .query_row(
            "SELECT available FROM native_stream_snapshots WHERE connection_id=?1",
            [connection],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .unwrap_or(false);
    if !available {
        return Ok(());
    }
    let mut value = load(c, connection)?;
    if value["stream_metadata_revision"] != expected
        || value["stream_metadata_generation"] != generation
    {
        return Ok(());
    }
    let mut out = records(&value)?;
    let counter = bump(
        value["stream_metadata_counter"]
            .as_u64()
            .ok_or_else(invalid)?,
    )?;
    for stream in streams {
        let Some(record) = out.iter_mut().find(|r| {
            r.id == stream.binding.stream_id
                && r.generation == stream.binding.stream_generation
                && r.medium == stream.binding.medium
        }) else {
            return Err(invalid());
        };
        if stream.binding.connection_generation != generation || record.state == "ended" {
            return Err(invalid());
        }
        record.state = stream.state.clone();
        record.revision = counter;
    }
    value["stream_metadata_counter"] = json!(counter);
    value["stream_metadata"] = serde_json::to_value(out).map_err(|_| invalid())?;
    value["stream_metadata_revision"] = json!(revision);
    save(c, connection, generation, revision, &value)
}
/// Carry unchanged descriptors only through a captured route commit on the
/// exact still-ready incarnation. Gate states come from that atomic commit.
pub(crate) fn audience(
    c: &Connection,
    connection: &str,
    generation: u64,
    expected: u64,
    revision: u64,
    gates: &[connect_protocol::native_audience::AudienceStreamState],
) -> Result<()> {
    if expected.checked_add(1) != Some(revision) || revision > MAX_REV {
        return Err(invalid());
    }
    let raw: Option<String> = c.query_row(
        "SELECT s.projection_json FROM native_stream_snapshots s JOIN connections p ON p.id=s.connection_id WHERE s.connection_id=?1 AND s.generation=?2 AND s.stream_revision=?3 AND s.available=1 AND p.generation=?2 AND p.state IN ('ready','active') AND json_extract(p.readiness_json,'$.stream_revision')=?4",
        params![connection,generation,expected,revision], |r|r.get(0),
    ).optional().map_err(|_|Error::storage())?;
    let Some(raw) = raw else {
        return Ok(());
    };
    let mut value: Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    if value["stream_metadata_generation"] != generation
        || value["stream_metadata_revision"] != expected
    {
        return Err(invalid());
    }
    let mut out = records(&value)?;
    if !gates.is_empty() {
        let counter = bump(
            value["stream_metadata_counter"]
                .as_u64()
                .ok_or_else(invalid)?,
        )?;
        for gate in gates {
            let Some(record) = out.iter_mut().find(|r| {
                r.id == gate.stream_id
                    && r.generation == gate.stream_generation
                    && r.medium == "audio"
                    && (r.state != "ended" || gate.state == "ended")
            }) else {
                // Preserve known effect attribution, but never relabel an
                // incomplete descriptor set as the current actual resources.
                c.execute("UPDATE native_stream_snapshots SET available=0 WHERE connection_id=?1 AND generation=?2 AND stream_revision=?3",params![connection,generation,expected]).map_err(|_|Error::storage())?;
                return Ok(());
            };
            record.state = gate.state.clone();
            record.revision = counter;
        }
        value["stream_metadata_counter"] = json!(counter);
        value["stream_metadata"] = serde_json::to_value(out).map_err(|_| invalid())?;
    }
    value["stream_metadata_revision"] = json!(revision);
    save(c, connection, generation, revision, &value)
}
pub(crate) fn snapshot(c: &Connection, parent: &ConnectionState) -> Result<Vec<StreamState>> {
    let row:Option<(u64,u64,bool,String)>=c.query_row("SELECT generation,stream_revision,available,projection_json FROM native_stream_snapshots WHERE connection_id=?1",[&parent.connection_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
    let Some((generation, revision, available, raw)) = row else {
        return Ok(vec![]);
    };
    let value: Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    let terminal = parent.state == "retired";
    if generation > parent.generation {
        return Err(invalid());
    }
    if !terminal {
        if !available
            || generation != parent.generation
            || !matches!(parent.state.as_str(), "ready" | "active")
        {
            return Ok(vec![]);
        }
        let current:Option<u64>=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![parent.connection_id,parent.generation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if current != Some(revision) {
            return Ok(vec![]);
        }
    }
    records(&value)?
        .into_iter()
        .map(|v| {
            Ok(StreamState {
                stream_id: v.id,
                session_id: parent.session_id.clone(),
                participant_id: parent.participant_id.clone(),
                connection_id: Some(parent.connection_id.clone()),
                medium: v.medium,
                revision: if terminal && v.state != "ended" {
                    bump(
                        value["stream_metadata_counter"]
                            .as_u64()
                            .ok_or_else(invalid)?,
                    )?
                } else {
                    v.revision
                },
                state: if terminal { "ended".into() } else { v.state },
            })
        })
        .collect()
}

/// Store control-plane fixtures do not run Core or claim gate/media evidence.
#[cfg(test)]
pub(crate) fn unchanged_audience_metadata_for_test(
    observation: &Value,
) -> connect_protocol::native_audience::AudienceMetadata {
    observation["receiver_revisions"]
        .as_object()
        .map(|values| values.keys().map(|id| (id.clone(), Vec::new())).collect())
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn binding(id: &str, generation: u64) -> CodecBinding {
        CodecBinding {
            stream_id: id.into(),
            medium: "audio".into(),
            direction: "sendrecv".into(),
            codec: json!({"name":"pcmu","params":{"sample_rate":8000,"channels":1}}),
            stream_local_id: 1,
            stream_generation: generation,
            connection_generation: 1,
        }
    }
    fn database(value: &Value) -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE connections(id TEXT PRIMARY KEY,generation INTEGER,readiness_json TEXT)",
        )
        .unwrap();
        c.execute(
            "INSERT INTO connections VALUES('connection',1,?1)",
            [value.to_string()],
        )
        .unwrap();
        c.execute_batch(include_str!(
            "../migrations/112_native_stream_snapshots.sql"
        ))
        .unwrap();
        save(
            &c,
            "connection",
            1,
            value["stream_metadata_revision"].as_u64().unwrap(),
            value,
        )
        .unwrap();
        c
    }
    fn parent() -> ConnectionState {
        ConnectionState {
            connection_id: "connection".into(),
            session_id: "session".into(),
            participant_id: "participant".into(),
            membership_epoch: 1,
            endpoint_id: "endpoint".into(),
            device_id: "device".into(),
            generation: 1,
            revision: 88,
            stream_revision: Some(1),
            state: "ready".into(),
            replaces: None,
        }
    }
    fn audience_database() -> Connection {
        let mut value = json!({"stream_revision":1});
        replace(
            &mut value,
            1,
            None,
            1,
            &[binding("voice", 1), binding("other", 1)],
        )
        .unwrap();
        let c = database(&value);
        c.execute_batch("ALTER TABLE connections ADD COLUMN state TEXT NOT NULL DEFAULT 'ready'")
            .unwrap();
        c
    }
    fn audience_revision(c: &Connection, revision: u64) {
        c.execute(
            "UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?1)",
            [revision],
        )
        .unwrap();
    }
    #[test]
    fn audience_carries_exact_descriptors_and_only_captured_gate_resource_versions() {
        use connect_protocol::native_audience::AudienceStreamState;
        let c = audience_database();
        let original = snapshot(&c, &parent()).unwrap();
        audience_revision(&c, 2);
        assert!(
            snapshot(&c, &parent()).unwrap().is_empty(),
            "an uncarried control revision cannot label old resources as current"
        );
        audience(&c, "connection", 1, 1, 2, &[]).unwrap();
        assert_eq!(
            snapshot(&c, &parent()).unwrap(),
            original,
            "route-only carry changes no resource state or version"
        );
        audience_revision(&c, 3);
        audience(
            &c,
            "connection",
            1,
            2,
            3,
            &[AudienceStreamState {
                stream_id: "voice".into(),
                stream_generation: 1,
                state: "paused".into(),
            }],
        )
        .unwrap();
        let observed = snapshot(&c, &parent()).unwrap();
        let voice = observed.iter().find(|s| s.stream_id == "voice").unwrap();
        let other = observed.iter().find(|s| s.stream_id == "other").unwrap();
        assert_eq!(voice.state, "paused");
        assert_eq!(voice.revision, 2);
        assert_eq!(
            other,
            original.iter().find(|s| s.stream_id == "other").unwrap()
        );
        let saved = load(&c, "connection").unwrap();
        audience(&c, "connection", 1, 2, 3, &[]).unwrap();
        assert_eq!(
            load(&c, "connection").unwrap(),
            saved,
            "original replay must not bump observations again"
        );
        assert!(records(&saved).unwrap().iter().all(|s| s.generation == 1));
        assert!(audience(&c, "connection", 1, 3, 5, &[]).is_err());
    }
    #[test]
    fn audience_never_revives_retired_unavailable_replaced_or_newer_descriptions() {
        for case in ["retired", "unavailable", "replaced", "newer"] {
            let c = audience_database();
            audience_revision(&c, 2);
            match case {
                "retired" => {
                    c.execute("UPDATE connections SET state='retired'", [])
                        .unwrap();
                }
                "unavailable" => {
                    c.execute("UPDATE native_stream_snapshots SET available=0", [])
                        .unwrap();
                }
                "replaced" => {
                    c.execute("UPDATE connections SET generation=2", [])
                        .unwrap();
                }
                "newer" => {
                    let mut value = load(&c, "connection").unwrap();
                    replace(&mut value, 1, Some(1), 3, &[binding("voice", 1)]).unwrap();
                    save(&c, "connection", 1, 3, &value).unwrap();
                }
                _ => unreachable!(),
            }
            let before:(u64,u64,bool,String)=c.query_row("SELECT generation,stream_revision,available,projection_json FROM native_stream_snapshots",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            audience(&c, "connection", 1, 1, 2, &[]).unwrap();
            let after:(u64,u64,bool,String)=c.query_row("SELECT generation,stream_revision,available,projection_json FROM native_stream_snapshots",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!(before, after, "{case}");
        }
    }
    #[test]
    fn audience_missing_or_wrong_stream_incarnation_makes_metadata_unavailable() {
        use connect_protocol::native_audience::AudienceStreamState;
        for (id, generation) in [("missing", 1), ("voice", 2)] {
            let c = audience_database();
            let before = load(&c, "connection").unwrap();
            audience_revision(&c, 2);
            audience(
                &c,
                "connection",
                1,
                1,
                2,
                &[AudienceStreamState {
                    stream_id: id.into(),
                    stream_generation: generation,
                    state: "active".into(),
                }],
            )
            .unwrap();
            assert_eq!(load(&c, "connection").unwrap(), before);
            assert!(!c
                .query_row("SELECT available FROM native_stream_snapshots", [], |r| {
                    r.get::<_, bool>(0)
                })
                .unwrap());
            assert!(snapshot(&c, &parent()).unwrap().is_empty());
        }
    }
    fn update(c: &Connection, value: &Value) {
        c.execute(
            "UPDATE connections SET readiness_json=?1",
            [value.to_string()],
        )
        .unwrap();
        save(
            c,
            "connection",
            1,
            value["stream_metadata_revision"].as_u64().unwrap(),
            value,
        )
        .unwrap();
    }
    #[test]
    fn captures_selected_bindings_with_distinct_resource_versions_and_no_transport_grant() {
        let mut value = json!({"stream_revision":7});
        replace(
            &mut value,
            1,
            None,
            7,
            &[binding("one", 1), binding("two", 1)],
        )
        .unwrap();
        let c = database(&value);
        let first = snapshot(&c, &parent()).unwrap();
        assert_eq!(first.len(), 2);
        assert!(first
            .iter()
            .all(|s| s.revision == 1 && s.state == "negotiated"));
        let serialized = serde_json::to_string(&first).unwrap();
        assert!(!serialized.contains("stream_local_id") && !serialized.contains("codec"));
        replace(
            &mut value,
            1,
            Some(7),
            8,
            &[binding("one", 1), binding("two", 2)],
        )
        .unwrap();
        value["stream_revision"] = json!(8);
        update(&c, &value);
        let current = snapshot(&c, &parent()).unwrap();
        assert_eq!(
            current
                .iter()
                .find(|s| s.stream_id == "one")
                .unwrap()
                .revision,
            1
        );
        assert_eq!(
            current
                .iter()
                .find(|s| s.stream_id == "two")
                .unwrap()
                .revision,
            2
        );
    }
    #[test]
    fn incomplete_capture_and_wrong_incarnation_cannot_relabel_old_metadata_as_current() {
        let mut value = json!({"stream_revision":1});
        replace(&mut value, 1, None, 1, &[binding("one", 1)]).unwrap();
        let c = database(&value);
        value["stream_revision"] = json!(2);
        update(&c, &value);
        assert!(snapshot(&c, &parent()).unwrap().is_empty());
        complete(&c, "connection", 1, 1, 2, None).unwrap();
        assert!(snapshot(&c, &parent()).unwrap().is_empty());
        let mut wrong = binding("one", 1);
        wrong.connection_generation = 2;
        assert!(replace(&mut value, 1, Some(1), 2, &[wrong]).is_err());
        let mut retired = parent();
        retired.state = "retired".into();
        retired.generation = 2;
        c.execute(
            "UPDATE connections SET generation=2,readiness_json=NULL",
            [],
        )
        .unwrap();
        let terminal = snapshot(&c, &retired).unwrap();
        assert_eq!(terminal[0].state, "ended");
        assert_eq!(terminal[0].revision, 2);
    }
    #[test]
    fn removal_is_terminal_and_reappearance_does_not_reset_a_resource_version_after_summary_eviction(
    ) {
        let mut value = json!({"stream_revision":1});
        replace(&mut value, 1, None, 1, &[binding("original", 1)]).unwrap();
        for revision in 2..100 {
            let b = binding(&format!("stream-{revision}"), 1);
            replace(&mut value, 1, Some(revision - 1), revision, &[b]).unwrap();
            value["stream_revision"] = json!(revision);
            assert!(records(&value).unwrap().len() <= MAX_RECORDS);
        }
        assert!(!records(&value).unwrap().iter().any(|s| s.id == "original"));
        replace(&mut value, 1, Some(99), 100, &[binding("original", 2)]).unwrap();
        value["stream_revision"] = json!(100);
        let c = database(&value);
        let current = snapshot(&c, &parent()).unwrap();
        let returned = current.iter().find(|s| s.stream_id == "original").unwrap();
        assert_eq!(returned.state, "negotiated");
        assert_eq!(returned.revision, 100);
        let mut terminal = parent();
        terminal.state = "retired".into();
        assert!(snapshot(&c, &terminal)
            .unwrap()
            .iter()
            .all(|s| s.state == "ended"));
        value["stream_metadata_counter"] = json!(MAX_REV);
        assert!(replace(&mut value, 1, Some(100), 101, &[binding("original", 3)]).is_err());
    }
}
