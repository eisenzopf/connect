//! Fixture custody for an actual current physical owner's public connection.end.
//! Input envelopes must already be verified by CanonicalClient. No IO or grant.
use serde_json::{json, Value};
#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Completed,
    Refused,
    Unknown,
    DeferredRead,
}
pub struct End {
    scope: [String; 3],
    operation: String,
    programme_end_ms: u64,
    cutoff_ms: u64,
    attempted: bool,
    initial: Option<String>,
    settled: bool,
    reading: Option<String>,
    reads: Vec<String>,
    update: Option<Value>,
    record: Option<String>,
    revision: u64,
    completed: Option<Value>,
    acknowledged: bool,
}
fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._~-".contains(&c))
}
impl End {
    pub fn new(
        scope: [&str; 3],
        operation: &str,
        programme_end_ms: u64,
    ) -> Result<Self, &'static str> {
        if !scope.iter().chain([operation].iter()).all(|v| id(v)) {
            return Err("invalid original end scope");
        }
        let cutoff_ms = programme_end_ms
            .checked_add(5000)
            .ok_or("original teardown cutoff overflow")?;
        Ok(Self {
            scope: scope.map(str::to_owned),
            operation: operation.into(),
            programme_end_ms,
            cutoff_ms,
            attempted: false,
            initial: None,
            settled: false,
            reading: None,
            reads: vec![],
            update: None,
            record: None,
            revision: 0,
            completed: None,
            acknowledged: false,
        })
    }
    pub fn intent(&self) -> Value {
        json!({"format":"connect/physical-owner-original-end/1","type":"connection.end",
            "scope":{"conversation_id":self.scope[0],"session_id":self.scope[1],"connection_id":self.scope[2],"operation_id":self.operation},
            "payload":{"reason_code":200,"reason":"Synthetic workload complete"},
            "programme_end_monotonic_ms":self.programme_end_ms,"original_cutoff_monotonic_ms":self.cutoff_ms})
    }
    fn within(&self, now: u64) -> Result<(), &'static str> {
        if now < self.programme_end_ms || now >= self.cutoff_ms {
            Err("original five-second teardown interval unavailable")
        } else {
            Ok(())
        }
    }
    /// The caller durably saves intent() before this first possible submission.
    /// A failed/unknown send retains attempted custody and can never re-enter.
    pub fn begin(&mut self, media_stopped: bool, now: u64) -> Result<Value, &'static str> {
        self.within(now)?;
        if !media_stopped || self.attempted {
            return Err("original owner end is unavailable for submission");
        }
        self.attempted = true;
        Ok(self.intent())
    }
    pub fn bind_submission(&mut self, request: &str, now: u64) -> Result<(), &'static str> {
        self.within(now)?;
        if !self.attempted || self.initial.is_some() || !id(request) {
            return Err("original end correlation unavailable");
        }
        self.initial = Some(request.into());
        Ok(())
    }
    /// An uncertain original send is recoverable by its saved logical key,
    /// including when no wire correlation returned. This never permits send reentry.
    pub fn submission_unknown(&mut self, now: u64) -> Result<(), &'static str> {
        self.within(now)?;
        if !self.attempted || self.settled || self.reading.is_some() || self.completed.is_some() {
            return Err("no uncertain original end submission is available");
        }
        self.settled = true;
        self.update = None;
        Ok(())
    }
    pub fn status_payload(&self) -> Value {
        json!({"operation_id":self.operation})
    }
    pub fn bind_read(&mut self, request: &str, now: u64) -> Result<(), &'static str> {
        self.within(now)?;
        if !self.settled
            || self.reading.is_some()
            || !id(request)
            || self.initial.as_deref() == Some(request)
            || self.reads.iter().any(|v| v == request)
            || self.reads.len() >= 128
        {
            return Err("original end read correlation unavailable");
        }
        self.reads.push(request.into());
        self.reading = Some(request.into());
        Ok(())
    }
    pub fn completed_with_ack(&self) -> bool {
        self.completed.is_some() && self.acknowledged
    }
    pub fn observe(&mut self, frame: &Value, now: u64) -> Result<Option<Progress>, &'static str> {
        self.within(now)?;
        let reading = self.reading.is_some();
        let request = if reading {
            self.reading.as_deref()
        } else if !self.settled {
            self.initial.as_deref()
        } else {
            None
        }
        .ok_or("no original end response is outstanding")?;
        if frame["v"] != 2
            || frame["cid"] != self.scope[0]
            || frame["sid"] != self.scope[1]
            || frame["connid"] != self.scope[2]
            || frame["in_reply_to"] != request
            || frame.get("expected_revision").is_some()
            || if reading {
                frame.get("operation_id").is_some()
            } else {
                frame["operation_id"] != self.operation
            }
        {
            return Err("original end response changed scope or correlation");
        }
        match frame["type"].as_str() {
            Some("operation.updated") => {
                let p = &frame["payload"];
                let record = p["record_id"]
                    .as_str()
                    .filter(|v| id(v))
                    .ok_or("original end record missing")?;
                let revision = p["revision"]
                    .as_u64()
                    .filter(|v| *v > 0 && *v >= self.revision && *v <= 9_007_199_254_740_991)
                    .ok_or("original end revision invalid or regressed")?;
                if self.update.is_some()
                    || p["operation_id"] != self.operation
                    || p["command_type"] != "connection.end"
                    || !p["expected_revision"].is_null()
                    || self.record.as_deref().is_some_and(|v| v != record)
                {
                    return Err("original end journal identity changed");
                }
                match p["state"].as_str() {
                    Some("completed") => {
                        let result = &p["result"];
                        if result["connection_id"] != self.scope[2]
                            || result["ended"] != true
                            || !p["problem"].is_null()
                            || self.completed.as_ref().is_some_and(|v| v != result)
                        {
                            return Err("original end result changed its current owner");
                        }
                    }
                    Some(
                        "accepted" | "preparing" | "ready" | "committing" | "failed" | "cancelled"
                        | "unknown",
                    ) if self.completed.is_none() => {}
                    _ => return Err("original end journal state invalid or regressed"),
                }
                self.record = Some(record.into());
                self.revision = revision;
                self.update = Some(p.clone());
                Ok(None)
            }
            Some("ack") => {
                let p = self
                    .update
                    .take()
                    .ok_or("original end ACK omitted its journal result")?;
                let progress = match p["state"].as_str() {
                    Some("completed") => {
                        self.completed = Some(p["result"].clone());
                        self.acknowledged = true;
                        Progress::Completed
                    }
                    Some("failed" | "cancelled") => Progress::Refused,
                    Some("unknown") => Progress::Unknown,
                    _ => Progress::Pending,
                };
                if reading {
                    self.reading = None;
                } else {
                    self.settled = true;
                }
                Ok(Some(progress))
            }
            Some("error")
                if reading && frame["payload"]["code"] == 503 && self.update.is_none() =>
            {
                self.reading = None;
                Ok(Some(Progress::DeferredRead))
            }
            _ => Err("unexpected original end response; no mutation retry"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> End {
        End::new(["room", "session", "current-owner"], "original-end", 62000).unwrap()
    }
    fn started() -> End {
        let mut v = fixture();
        v.begin(true, 62000).unwrap();
        v.bind_submission("send-wire", 62000).unwrap();
        v
    }
    fn frame(kind: &str, read: bool) -> Value {
        let mut v = json!({"v":2,"type":kind,"cid":"room","sid":"session","connid":"current-owner","in_reply_to":if read{"read-wire"}else{"send-wire"},"payload":{}});
        if !read {
            v["operation_id"] = json!("original-end");
        }
        v
    }
    fn update(state: &str, read: bool, revision: u64) -> Value {
        let mut v = frame("operation.updated", read);
        v["payload"] = json!({"operation_id":"original-end","record_id":"record-one","command_type":"connection.end","expected_revision":null,"revision":revision,"state":state});
        if state == "completed" {
            v["payload"]["result"] = json!({"connection_id":"current-owner","ended":true});
        }
        v
    }
    #[test]
    fn media_stops_before_once_only_end_and_unknown_submission_never_reenters() {
        let mut v = fixture();
        assert!(v.begin(true, 61999).is_err());
        assert!(v.begin(false, 62000).is_err());
        let saved = v.intent();
        assert_eq!(v.begin(true, 62000).unwrap(), saved);
        assert!(v.begin(true, 62001).is_err());
        assert!(v.bind_read("read-wire", 62001).is_err());
    }
    #[test]
    fn completed_current_owner_needs_its_own_ack() {
        let mut v = started();
        assert_eq!(v.observe(&update("completed", false, 1), 62001), Ok(None));
        assert!(!v.completed_with_ack());
        assert_eq!(
            v.observe(&frame("ack", false), 62002),
            Ok(Some(Progress::Completed))
        );
        assert!(v.completed_with_ack());
        assert!(v.observe(&frame("ack", false), 62003).is_err());
        assert!(v.begin(true, 62003).is_err());
    }
    #[test]
    fn every_foreign_scope_and_result_refuses_without_completion() {
        for key in ["cid", "sid", "connid", "operation_id", "in_reply_to"] {
            let mut v = started();
            let mut f = update("completed", false, 1);
            f[key] = json!("foreign");
            assert!(v.observe(&f, 62001).is_err(), "{key}");
            assert!(!v.completed_with_ack());
        }
        for key in ["connection_id", "ended"] {
            let mut v = started();
            let mut f = update("completed", false, 1);
            f["payload"]["result"][key] = json!("foreign");
            assert!(v.observe(&f, 62001).is_err());
        }
    }
    #[test]
    fn pending_reads_retain_original_key_and_unchanged_cutoff() {
        let mut v = started();
        v.observe(&update("preparing", false, 2), 62001).unwrap();
        assert_eq!(
            v.observe(&frame("ack", false), 62002),
            Ok(Some(Progress::Pending))
        );
        assert_eq!(v.status_payload(), json!({"operation_id":"original-end"}));
        let original = v.intent();
        v.bind_read("read-wire", 63000).unwrap();
        v.observe(&update("completed", true, 3), 63001).unwrap();
        assert_eq!(
            v.observe(&frame("ack", true), 63002),
            Ok(Some(Progress::Completed))
        );
        assert_eq!(v.intent(), original);
        assert!(v.bind_read("too-late", 67000).is_err());
        assert!(v.begin(true, 66000).is_err());
    }
    #[test]
    fn read_503_is_observation_only_and_never_renews_five_seconds() {
        let mut v = started();
        v.observe(&update("accepted", false, 1), 62000).unwrap();
        v.observe(&frame("ack", false), 62000).unwrap();
        v.bind_read("read-wire", 66000).unwrap();
        let mut f = frame("error", true);
        f["payload"] = json!({"code":503});
        assert_eq!(v.observe(&f, 66999), Ok(Some(Progress::DeferredRead)));
        assert!(v.bind_read("read-wire", 66999).is_err());
        assert!(v.bind_read("new-read", 67000).is_err());
        assert!(!v.completed_with_ack());
    }
    #[test]
    fn recovery_refuses_changed_record_and_regressed_revision() {
        for change in [0, 1] {
            let mut v = started();
            v.observe(&update("preparing", false, 3), 62000).unwrap();
            v.observe(&frame("ack", false), 62000).unwrap();
            v.bind_read("read-wire", 62001).unwrap();
            let mut f = update("completed", true, 4);
            if change == 0 {
                f["payload"]["record_id"] = json!("foreign");
            } else {
                f["payload"]["revision"] = json!(2);
            }
            assert!(v.observe(&f, 62002).is_err());
            assert!(!v.completed_with_ack());
        }
    }
    #[test]
    fn refused_or_unknown_end_does_not_authorize_another_mutation() {
        for (state, progress) in [
            ("failed", Progress::Refused),
            ("unknown", Progress::Unknown),
        ] {
            let mut v = started();
            v.observe(&update(state, false, 1), 62000).unwrap();
            assert_eq!(v.observe(&frame("ack", false), 62001), Ok(Some(progress)));
            assert!(!v.completed_with_ack());
            assert!(v.begin(true, 62002).is_err());
        }
    }
    #[test]
    fn uncertain_send_without_wire_receipt_recovers_only_the_saved_original_key() {
        let mut v = fixture();
        assert!(v.submission_unknown(62000).is_err());
        v.begin(true, 62000).unwrap();
        v.submission_unknown(62001).unwrap();
        assert!(v.begin(true, 62002).is_err());
        assert!(v.submission_unknown(62002).is_err());
        assert_eq!(v.status_payload(), json!({"operation_id":"original-end"}));
        v.bind_read("read-wire", 62002).unwrap();
        v.observe(&update("completed", true, 2), 62003).unwrap();
        assert_eq!(
            v.observe(&frame("ack", true), 62004),
            Ok(Some(Progress::Completed))
        );
        assert!(v.completed_with_ack());
    }
    #[test]
    fn result_or_ack_after_original_cutoff_never_counts_as_teardown_pass() {
        let mut v = started();
        v.observe(&update("completed", false, 1), 66999).unwrap();
        assert!(v.observe(&frame("ack", false), 67000).is_err());
        assert!(!v.completed_with_ack());
        assert!(End::new(["room", "session", "owner"], "end", u64::MAX).is_err());
    }
}
