//! Test-only passive private evidence. No media/policy/clock operation changes.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
const FILE_LIMIT: u64 = 16 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 64 * 1024 * 1024;

pub(super) struct Capture {
    source: PathBuf,
    database: PathBuf,
    destination: Option<PathBuf>,
    saved: bool,
    browser: Option<BrowserOwner>,
}
fn private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|e| e.to_string())
}
impl Capture {
    pub(super) fn new(root: &Path, source: PathBuf, database: PathBuf) -> Self {
        let destination =
            std::env::var_os("CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS").map(|value| {
                let parent = PathBuf::from(value)
                    .canonicalize()
                    .expect("owned artifact case directory exists");
                let ignored = root.join(".local").canonicalize().unwrap();
                assert!(
                    parent.starts_with(ignored),
                    "private evidence belongs under ignored original root"
                );
                let destination = parent.join("fixture");
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder
                    .create(&destination)
                    .expect("fresh exclusive artifact directory");
                destination
            });
        Self {
            source,
            database,
            destination,
            saved: false,
            browser: None,
        }
    }
    /// Called only AFTER original cleanup attempts, before any verdict escapes.
    /// Drop is a partial diagnostic fallback for earlier setup panic, not a join.
    pub(super) fn save(&mut self, stage: &str) -> Result<(), String> {
        if self.saved {
            return Ok(());
        }
        self.saved = true;
        let Some(destination) = &self.destination else {
            return Ok(());
        };
        let mut files = Vec::new();
        let mut errors = Vec::new();
        let mut total = 0u64;
        if self.source.is_dir() {
            match fs::read_dir(&self.source) {
                Ok(entries) => {
                    for entry in entries {
                        let result = (|| -> Result<(), String> {
                            let entry = entry.map_err(|e| e.to_string())?;
                            let name = entry
                                .file_name()
                                .into_string()
                                .map_err(|_| "non-UTF8 evidence filename")?;
                            // No profile/CA/private actor input or encoder/raw-media copy.
                            if name == "input.json"
                                || name == "private-input.json"
                                || !(name.ends_with(".json") || name.ends_with(".log"))
                            {
                                return Ok(());
                            }
                            let metadata = entry.metadata().map_err(|e| e.to_string())?;
                            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                                return Ok(());
                            }
                            if files.len() >= 128 || metadata.len() > FILE_LIMIT {
                                return Err("bounded evidence file count/size".into());
                            }
                            let mut bytes = Vec::new();
                            fs::File::open(entry.path())
                                .map_err(|e| e.to_string())?
                                .take(FILE_LIMIT + 1)
                                .read_to_end(&mut bytes)
                                .map_err(|e| e.to_string())?;
                            if bytes.len() as u64 != metadata.len()
                                || bytes.len() as u64 > FILE_LIMIT
                            {
                                return Err("evidence file changed or exceeded bound".into());
                            }
                            total = total
                                .checked_add(bytes.len() as u64)
                                .ok_or("evidence size overflow")?;
                            if total > TOTAL_LIMIT {
                                return Err("bounded total evidence bytes".into());
                            }
                            private_file(&destination.join(&name), &bytes)?;
                            files.push(json!({"name":name,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
                            Ok(())
                        })();
                        if let Err(error) = result {
                            errors.push(error);
                        }
                        if errors.len() >= 16 {
                            errors.push("further evidence errors omitted".into());
                            break;
                        }
                    }
                }
                Err(error) => errors.push(error.to_string()),
            }
        } else {
            errors.push("fixture evidence directory not created before failure".into());
        }
        // Immutable diagnostic SQL snapshot, external READONLY/no queued Store
        // mutex/authority acquisition. No retry or original deadline renewal.
        let ledger = (|| -> Result<Value, String> {
            let connection = rusqlite::Connection::open_with_flags(
                &self.database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|e| e.to_string())?;
            connection
                .busy_timeout(Duration::ZERO)
                .map_err(|e| e.to_string())?;
            connection
                .execute_batch("BEGIN")
                .map_err(|e| e.to_string())?;
            let mut statement = connection.prepare("SELECT operation_record_id,phase,ordinal,phase_json FROM native_replacement_public_video_phases ORDER BY operation_record_id,ordinal LIMIT 65").map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            let mut phases = Vec::new();
            for row in rows {
                let (record, phase, ordinal, raw) = row.map_err(|e| e.to_string())?;
                if phases.len() >= 64 || raw.len() > 16384 {
                    return Err("bounded phase evidence".into());
                }
                let actual: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
                phases.push(json!({"operation_record_id":record,"phase":phase,"ordinal":ordinal,"value":actual}));
            }
            drop(statement);
            let mut statement = connection.prepare("SELECT o.record_id,o.operation_id,o.state,o.revision FROM operations o JOIN native_replacement_public_video_reservations r ON r.operation_record_id=o.record_id ORDER BY o.record_id LIMIT 17").map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u64>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            let mut operations = Vec::new();
            for row in rows {
                let (record, id, state, revision) = row.map_err(|e| e.to_string())?;
                if operations.len() >= 16 {
                    return Err("bounded operation evidence".into());
                }
                operations.push(
                    json!({"record_id":record,"operation_id":id,"state":state,"revision":revision}),
                );
            }
            drop(statement);
            connection
                .execute_batch("ROLLBACK")
                .map_err(|e| e.to_string())?;
            Ok(
                json!({"profile":"connect/actual-public-video-ledger-snapshot/1","read_only":true,"phases":phases,"operations":operations}),
            )
        })();
        match ledger {
            Ok(value) => {
                let written = (|| -> Result<(), String> {
                    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
                    total = total
                        .checked_add(bytes.len() as u64)
                        .ok_or("ledger size overflow")?;
                    if bytes.len() > 2 * 1024 * 1024 || total > TOTAL_LIMIT {
                        return Err("bounded ledger/total evidence bytes".into());
                    }
                    private_file(&destination.join("actual-ledger.json"), &bytes)?;
                    files.push(json!({"name":"actual-ledger.json","bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
                    Ok(())
                })();
                if let Err(error) = written {
                    errors.push(error);
                }
            }
            Err(error) => errors.push(error),
        }
        files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let complete = errors.is_empty();
        let value = json!({"profile":"connect/private-public-b-evidence-archive/1","stage":stage,"complete":complete,
            "files":files,"errors":errors,"source_bytes":total,"closure_claim":false,
            "scope":"Actual private files and READONLY phase snapshot only; original test verdict/joins independently required"});
        private_file(
            &destination.join("capture.json"),
            &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )?;
        if complete {
            Ok(())
        } else {
            Err("private public B evidence archive incomplete; inspect capture.json".into())
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        if !self.saved {
            let _ = self.save("early-drop-partial-no-closure-claim");
        }
    }
}

// Numeric process-table evidence only: no arguments, environment or unrelated
// process names are retained. The original Child is NOT reaped before group
// decisions, so its allocated PID anchors the original PGID against reuse.
#[derive(Clone, Copy)]
struct BrowserOwner {
    pid: u32,
    pgid: u32,
}
#[derive(Clone)]
struct ProcessRow {
    pid: u32,
    pgid: u32,
    zombie: bool,
}
async fn process_rows(until: tokio::time::Instant) -> Result<Vec<ProcessRow>, String> {
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new("/bin/ps");
    command
        .args(["-axo", "pid=,pgid=,stat="])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|e| format!("numeric ps spawn: {e}"))?;
    let mut output = child.stdout.take().ok_or("numeric ps stdout absent")?;
    let result = tokio::time::timeout_at(until, async {
        let mut bytes = Vec::new();
        output
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("numeric ps output bound".into());
        }
        if !child.wait().await.map_err(|e| e.to_string())?.success() {
            return Err("numeric ps failed".into());
        }
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        for line in text.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 3 || rows.len() >= 32768 {
                return Err("numeric ps row shape/count".into());
            }
            let pid = fields[0].parse::<u32>().map_err(|e| e.to_string())?;
            let pgid = fields[1].parse::<u32>().map_err(|e| e.to_string())?;
            if fields[2].is_empty() || !fields[2].is_ascii() {
                return Err("numeric ps invalid identity/state".into());
            }
            rows.push(ProcessRow {
                pid,
                pgid,
                zombie: fields[2].starts_with('Z'),
            });
        }
        Ok(rows)
    })
    .await
    .map_err(|_| "numeric ps exceeded original cutoff".to_string());
    // Every helper subprocess is joined even on bound/error. This cleanup never
    // changes the original Chromium qualification cutoff.
    if !matches!(&result, Ok(Ok(_))) {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result?
}
impl Capture {
    pub(super) async fn browser_started(
        &mut self,
        child: &mut std::process::Child,
    ) -> Result<(), String> {
        let pid = child.id();
        // No try_wait here: preserve the Child until all group decisions finish.
        let rows = process_rows(tokio::time::Instant::now() + Duration::from_secs(2)).await?;
        let row = rows
            .iter()
            .find(|r| r.pid == pid)
            .ok_or("original Chromium PID absent at capture")?;
        if row.zombie || row.pgid != pid {
            return Err("original Chromium not live in its own actual group".into());
        }
        let owner = BrowserOwner {
            pid,
            pgid: row.pgid,
        };
        self.browser = Some(owner);
        private_file(
            &self.source.join("original-browser-owner.json"),
            &serde_json::to_vec_pretty(&json!({
                "profile":"connect/original-chromium-owner/1","pid":owner.pid,"pgid":owner.pgid,
                "observed_while_original_child_live":true,"pgid_inferred":false
            }))
            .map_err(|e| e.to_string())?,
        )
    }
    pub(super) async fn join_browser(
        &mut self,
        child: &mut std::process::Child,
        disarmed: &mut bool,
    ) -> Result<(), String> {
        if *disarmed {
            return Err("original browser already reaped/disarmed; no second group signal".into());
        }
        let began = tokio::time::Instant::now();
        let until = began + Duration::from_secs(5); // replaces the original timeout, never renewed
        let owner = self
            .browser
            .ok_or("original Chromium owner was not captured")?;
        if child.id() != owner.pid {
            return Err("different Chromium Child at original cleanup".into());
        }
        let mut reaped = false;
        let mut success = false;
        let mut absent = false;
        let mut exit_code = None;
        let natural = async {
            loop {
                let rows = process_rows(until).await?;
                let Some(leader) = rows.iter().find(|r| r.pid == owner.pid) else {
                    *disarmed = true; // ownership unknown: never signal old PGID
                    return Err("unreaped original Chromium leader absent".into());
                };
                if leader.pgid != owner.pgid {
                    *disarmed = true;
                    return Err("original Chromium group changed".to_string());
                }
                let group: Vec<_> = rows.iter().filter(|r| r.pgid == owner.pgid).collect();
                // Even zombie workers must disappear; only the original unreaped
                // leader is allowed before the actual Child wait.
                if leader.zombie && group.len() == 1 {
                    let status = match child.wait() {
                        Ok(status) => status,
                        Err(error) => {
                            *disarmed = true;
                            return Err(format!(
                                "original Chromium wait ownership unknown: {error}"
                            ));
                        }
                    };
                    // Actual owning wrapper flag: persist BEFORE any await or
                    // fallible evidence write, including cancellation/unwind.
                    *disarmed = true;
                    reaped = true;
                    success = status.success();
                    exit_code = status.code();
                    let rows = process_rows(until).await?;
                    absent = !rows.iter().any(|r| r.pgid == owner.pgid);
                    if !success || !absent {
                        return Err(
                            "Chromium status unsuccessful or original group still present".into(),
                        );
                    }
                    return Ok::<(), String>(());
                }
                tokio::time::sleep_until(
                    (tokio::time::Instant::now() + Duration::from_millis(20)).min(until),
                )
                .await;
            }
        }
        .await;
        let mut forced = false;
        if natural.is_err() && !reaped && !*disarmed {
            // The exact original Child is still unreaped and anchors this PGID.
            // This is failed-case cleanup only; it can NEVER qualify naturally.
            forced = true;
            let _ = std::process::Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", owner.pgid)])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            let _ = child.kill();
            match child.wait() {
                Ok(status) => {
                    *disarmed = true;
                    reaped = true;
                    success = status.success();
                    exit_code = status.code();
                }
                Err(_) => {
                    *disarmed = true;
                } // unknown ownership, never signal again
            }
            // No post-reap signal, and no renewed group-absence deadline.
        }
        let within = tokio::time::Instant::now() <= until;
        let qualified =
            natural.is_ok() && !forced && reaped && *disarmed && success && absent && within;
        let value = json!({"profile":"connect/original-chromium-closure/1","pid":owner.pid,"pgid":owner.pgid,
            "original_cleanup_seconds":5,"deadline_renewed":false,"elapsed_seconds":began.elapsed().as_secs_f64(),
            "child_wait_returned":reaped,"owning_drop_disarmed_at_reap":reaped && *disarmed,"exit_code":exit_code,"exit_success":success,"forced":forced,"group_absent":absent,
            "absence_observed_within_original_cutoff":absent && within,"qualified":qualified,
            "error":natural.err(),"separate_from_rust_group":true});
        private_file(
            &self.source.join("original-browser-closure.json"),
            &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )?;
        if qualified {
            Ok(())
        } else {
            Err("original Chromium group did not close naturally within original cutoff; inspect original-browser-closure.json".into())
        }
    }
}
