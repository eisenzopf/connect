//! Compare fresh indexed existence checks with the authoritative schema views.
use super::*;

fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    // Minimal relational inputs isolate audience algebra, including corrupt
    // cross-tenant rows that a current read must refuse. Public Store tests
    // separately exercise real-schema mutations and receipt authorization.
    c.execute_batch("CREATE TABLE identities(id TEXT PRIMARY KEY,tenant_id TEXT,revoked INTEGER);
        CREATE TABLE conversations(id TEXT PRIMARY KEY,tenant_id TEXT);
        CREATE TABLE participants(participant_id TEXT,conversation_id TEXT,identity_id TEXT,tenant_id TEXT,kind TEXT DEFAULT 'human',role TEXT DEFAULT 'participant',membership_epoch INTEGER,active INTEGER,removal_reason TEXT,PRIMARY KEY(conversation_id,identity_id));
        CREATE TABLE resource_audiences(kind TEXT,id TEXT,conversation_id TEXT,participant_id TEXT,membership_epoch INTEGER,PRIMARY KEY(kind,id,participant_id,membership_epoch));
        CREATE TABLE message_audiences(message_id TEXT,conversation_id TEXT,participant_id TEXT,membership_epoch INTEGER);
        CREATE TABLE artifact_audiences(artifact_id TEXT,conversation_id TEXT,participant_id TEXT,membership_epoch INTEGER);
        CREATE TABLE history_shares(id TEXT PRIMARY KEY,active INTEGER);
        CREATE TABLE history_share_items(sharing_id TEXT,kind TEXT,source_id TEXT,conversation_id TEXT,participant_id TEXT,membership_epoch INTEGER);
        INSERT INTO conversations VALUES('room','tenant'),('other','other-tenant');
        INSERT INTO identities VALUES('alice','tenant',0),('bob','tenant',0),('revoked','tenant',1),('foreign','other-tenant',0),('inactive','tenant',0);
        INSERT INTO participants(participant_id,conversation_id,identity_id,tenant_id,membership_epoch,active) VALUES
          ('a','room','alice','tenant',1,1),('b','room','bob','tenant',1,1),
          ('r','room','revoked','tenant',1,1),('f','room','foreign','other-tenant',1,1),('i','room','inactive','tenant',1,0);
        INSERT INTO history_shares VALUES('sharing',1);").unwrap();
    let migration = include_str!("../../migrations/021_membership_audiences.sql");
    let views = migration
        .split("CREATE VIEW original_content_audiences AS")
        .nth(1)
        .unwrap()
        .split("-- Compound events")
        .next()
        .unwrap();
    c.execute_batch(&format!("CREATE VIEW original_content_audiences AS{views}"))
        .unwrap();
    for kind in kinds() {
        for pid in ["a", "r", "f", "i"] {
            c.execute(
                "INSERT INTO resource_audiences VALUES(?1,'original','room',?2,1)",
                params![kind.as_str(), pid],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO resource_audiences VALUES(?1,'old-epoch','room','a',2)",
            [kind.as_str()],
        )
        .unwrap();
        c.execute(
            "INSERT INTO history_share_items VALUES('sharing',?1,'history-only','room','b',1)",
            [kind.as_str()],
        )
        .unwrap();
    }
    for table in ["message_audiences", "artifact_audiences"] {
        c.execute(
            &format!(
                "INSERT INTO {table} VALUES('branch-only','room','a',1),('original','room','a',1)"
            ),
            [],
        )
        .unwrap();
    }
    c
}
fn kinds() -> [ContentKind; 9] {
    [
        ContentKind::Message,
        ContentKind::Artifact,
        ContentKind::Session,
        ContentKind::Call,
        ContentKind::SipLeg,
        ContentKind::ContextDelivery,
        ContentKind::Record,
        ContentKind::Transcript,
        ContentKind::Observation,
    ]
}
fn reference(
    c: &Connection,
    identity: &str,
    room: &str,
    kind: ContentKind,
    id: &str,
    history: bool,
) -> Result<bool> {
    if !history {
        member(c, identity, room, true)?;
    }
    let view = if history {
        "content_audiences"
    } else {
        "original_content_audiences"
    };
    c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {view} a JOIN participants p ON p.participant_id=a.participant_id AND p.conversation_id=a.conversation_id AND p.membership_epoch=a.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id AND i.revoked=0 JOIN conversations v ON v.id=p.conversation_id AND v.tenant_id=p.tenant_id WHERE a.kind=?1 AND a.id=?2 AND a.conversation_id=?3 AND p.identity_id=?4)"),params![kind.as_str(),id,room,identity],|r|r.get(0)).map_err(|_|Error::storage())
}
fn check(c: &Connection) {
    for kind in kinds() {
        for identity in ["alice", "bob", "revoked", "foreign", "inactive", "missing"] {
            for room in ["room", "other", "missing"] {
                for id in [
                    "original",
                    "old-epoch",
                    "history-only",
                    "branch-only",
                    "missing",
                ] {
                    for history in [false, true] {
                        let expected =
                            reference(c, identity, room, kind, id, history).map_err(|e| e.code);
                        let actual = if history {
                            visible_to_identity(c, identity, room, kind, id)
                        } else {
                            original_to_identity(c, identity, room, kind, id)
                        }
                        .map_err(|e| e.code);
                        assert_eq!(
                            actual, expected,
                            "{identity}/{room}/{kind:?}/{id}/history={history}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn indexed_audience_matches_schema_views_for_all_kinds_epochs_and_tenants() {
    let c = fixture();
    check(&c);
    c.execute(
        "UPDATE participants SET membership_epoch=2 WHERE identity_id='alice'",
        [],
    )
    .unwrap();
    check(&c);
    c.execute("UPDATE history_shares SET active=0", []).unwrap();
    check(&c);
}
#[test]
fn indexed_audience_rechecks_history_revocation_membership_and_identity_after_statement_reuse() {
    let c = fixture();
    let kind = ContentKind::Session;
    assert!(visible_to_identity(&c, "bob", "room", kind, "history-only").unwrap());
    assert!(!original_to_identity(&c, "bob", "room", kind, "history-only").unwrap());
    c.execute("UPDATE history_shares SET active=0", []).unwrap();
    assert!(!visible_to_identity(&c, "bob", "room", kind, "history-only").unwrap());
    c.execute("UPDATE history_shares SET active=1", []).unwrap();
    c.execute(
        "UPDATE participants SET membership_epoch=2 WHERE identity_id='bob'",
        [],
    )
    .unwrap();
    assert!(!visible_to_identity(&c, "bob", "room", kind, "history-only").unwrap());
    c.execute("UPDATE history_share_items SET membership_epoch=2", [])
        .unwrap();
    assert!(visible_to_identity(&c, "bob", "room", kind, "history-only").unwrap());
    c.execute("UPDATE identities SET revoked=1 WHERE id='bob'", [])
        .unwrap();
    assert!(!visible_to_identity(&c, "bob", "room", kind, "history-only").unwrap());
    assert_eq!(
        original_to_identity(&c, "bob", "room", kind, "history-only")
            .unwrap_err()
            .code,
        "forbidden"
    );
    c.execute("UPDATE identities SET revoked=0 WHERE id='bob'", [])
        .unwrap();
    c.execute(
        "UPDATE participants SET active=0 WHERE identity_id='bob'",
        [],
    )
    .unwrap();
    assert!(!visible_to_identity(&c, "bob", "room", kind, "history-only").unwrap());
    assert_eq!(
        original_to_identity(&c, "bob", "room", kind, "history-only")
            .unwrap_err()
            .code,
        "forbidden"
    );
}
