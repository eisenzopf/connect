//! Pure SQLite representation controls. These do not mint an authority permit.
use super::*;

fn sample() -> Row {
    let mut row = Row::new(
        Table::Connections,
        vec![Value::Null; Table::Connections.columns().len()],
    )
    .unwrap();
    row.set("id", Value::Text("typed-tuple".into())).unwrap();
    row.set("generation", Value::Integer(0)).unwrap();
    row.set(
        "readiness_json",
        Value::Text(r#"{"stream_revision":1,"streams":["audio","video"]}"#.into()),
    )
    .unwrap();
    row
}

#[test]
fn scoped_tuples_preserve_json_text_bytes_and_sqlite_storage_classes() {
    let c = Connection::open_in_memory().unwrap();
    let exact = sample();
    let encoded = row_tuple(&c, &exact).unwrap();
    let original = text(&exact, "readiness_json").unwrap();
    let subtype_is_real: bool = c
        .query_row(
            "SELECT json_array(json(?1)) != json_array(json(?1)||'')",
            [original],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        subtype_is_real,
        "SQLite subtype must be present at the tested expression"
    );
    let direct = format!("SELECT {} = {}", typed("json(?1)"), typed("?1"));
    assert!(
        c.query_row(&direct, [original], |r| r.get::<_, bool>(0))
            .unwrap(),
        "direct JSON-subtyped expression differs from byte-identical TEXT"
    );
    for changed in [
        format!(" {original}"),
        r#"{"streams":["audio","video"],"stream_revision":1}"#.into(),
    ] {
        assert_ne!(changed, original);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&changed).unwrap(),
            serde_json::from_str::<serde_json::Value>(original).unwrap()
        );
        let mut row = exact.clone();
        row.set("readiness_json", Value::Text(changed)).unwrap();
        assert_ne!(row_tuple(&c, &row).unwrap(), encoded);
    }
    let mut encodings = std::collections::BTreeSet::new();
    for value in [
        Value::Integer(0),
        Value::Real(0.0),
        Value::Text("0".into()),
        Value::Blob(b"0".to_vec()),
        Value::Null,
        Value::Text("null".into()),
    ] {
        let mut row = exact.clone();
        row.set("generation", value).unwrap();
        assert!(
            encodings.insert(row_tuple(&c, &row).unwrap()),
            "SQLite storage classes collapsed"
        );
    }
    // Actual json() subtype on the trigger side must compare equal to the
    // bound exact TEXT bytes, without parsing or canonicalizing those bytes.
    let mut values = Vec::new();
    let columns = Table::Connections
        .columns()
        .iter()
        .enumerate()
        .map(|(index, name)| {
            values.push(exact.get(name).unwrap().clone());
            if *name == "readiness_json" {
                format!("json(?{}) AS \"{name}\"", index + 1)
            } else {
                format!("?{} AS \"{name}\"", index + 1)
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT {} FROM (SELECT {columns}) q",
        trigger_tuple(Table::Connections, "q")
    );
    let trigger: String = c
        .query_row(&sql, rusqlite::params_from_iter(values.iter()), |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        trigger, encoded,
        "json subtype differed from exact bound TEXT"
    );
}

#[test]
fn scoped_transitions_reject_delete_primary_key_and_table_changes() {
    let c = Connection::open_in_memory().unwrap();
    let old = sample();
    assert!(
        Transition::new(&c, Some(&old), None).is_err(),
        "DELETE obtained a transition"
    );
    let mut foreign = old.clone();
    foreign
        .set("id", Value::Text("foreign-connection".into()))
        .unwrap();
    assert!(
        Transition::new(&c, Some(&old), Some(&foreign)).is_err(),
        "primary key changed"
    );
    let different = Row::new(
        Table::Sessions,
        vec![Value::Null; Table::Sessions.columns().len()],
    )
    .unwrap();
    assert!(
        Transition::new(&c, Some(&old), Some(&different)).is_err(),
        "table changed"
    );
    let mut same = old.clone();
    same.set("readiness_json", Value::Text("{}".into()))
        .unwrap();
    assert!(
        Transition::new(&c, Some(&old), Some(&same)).is_ok(),
        "representation constructor is not the semantic authority gate"
    );
}
