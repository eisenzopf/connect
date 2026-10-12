//! Point samples of the whole qualification process, not server-only memory.
#![allow(clippy::unwrap_used)]
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub struct Metrics {
    path: PathBuf,
    started: Instant,
    observations: Vec<Value>,
    retired_digest: Option<String>,
}
impl Metrics {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("observations.json"),
            started: Instant::now(),
            observations: vec![],
            retired_digest: None,
        }
    }
    pub async fn capture(&mut self, database: &Path, completed: usize, live: usize, stage: &str) {
        let c = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (bindings, nonces, unknown): (usize, usize, usize) = c.query_row(
            "SELECT COUNT(*),COUNT(DISTINCT nonce),COUNT(*) FILTER(WHERE binding_json IS NULL) FROM vapi_session_setups",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!((bindings, nonces, unknown), (completed, completed, 0));
        let digest = if completed == 10_000 && live == 0 {
            let mut hash = Sha256::new();
            let mut query=c.prepare("SELECT session_id,nonce,binding_json FROM vapi_session_setups ORDER BY session_id").unwrap();
            let rows = query
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .unwrap();
            for row in rows {
                let bytes = serde_json::to_vec(&row.unwrap()).unwrap();
                hash.update((bytes.len() as u64).to_be_bytes());
                hash.update(bytes);
            }
            let digest = format!("{:x}", hash.finalize());
            if let Some(original) = &self.retired_digest {
                assert_eq!(&digest, original);
            } else {
                self.retired_digest = Some(digest.clone());
            }
            Some(digest)
        } else {
            None
        };
        drop(c);
        let output = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(output.status.success());
        let rss: u64 = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(rss > 0);
        let database_bytes: u64 = [
            database.to_owned(),
            PathBuf::from(format!("{}-wal", database.display())),
        ]
        .iter()
        .map(|p| std::fs::metadata(p).map_or(0, |m| m.len()))
        .sum();
        self.observations.push(json!({"stage":stage,"binding_digest":digest,"completed_setups":completed,"live_listeners":live,
            "bindings":bindings,"distinct_nonces":nonces,"unknown_bindings":unknown,
            "process_rss_bytes":rss*1024,"database_apparent_bytes":database_bytes,
            "elapsed_seconds":self.started.elapsed().as_secs_f64()}));
        self.save("running");
    }
    pub fn finish(&self) {
        self.save("passed");
    }
    fn save(&self, status: &str) {
        let value = json!({"status":status,"target_setups":10_000,"listener_limit":32,
            "setup_lifetime_seconds":5,"process_rss_scope":"Whole isolated Rust test process: server, clients and local API fixture. Point samples at batch boundaries; excludes Cargo/compiler and can miss transient peaks.",
            "observations":self.observations,"elapsed_seconds":self.started.elapsed().as_secs_f64()});
        let temporary = self.path.with_extension("tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        std::fs::rename(temporary, &self.path).unwrap();
    }
}
