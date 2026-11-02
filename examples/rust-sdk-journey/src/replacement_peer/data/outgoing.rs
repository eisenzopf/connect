//! Original Data command custody over already verified public-client replies.
//! No service IO occurs here; a retry may only be a read of this same operation.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Progress {
    Pending,
    CoreAccepted,
    Refused,
    DeferredRead,
}
pub(super) struct Outgoing {
    data: ExpectedData,
    attempted: bool,
    initial_id: Option<String>,
    initial_settled: bool,
    initial_update: Option<Value>,
    read_id: Option<String>,
    read_update: Option<Value>,
    read_ids: Vec<String>,
    record: Option<String>,
    revision: u64,
    snapshot: Option<Value>,
    accepted_result: Option<Value>,
    recovered: bool,
}
impl Outgoing {
    pub(super) fn new(data: ExpectedData) -> Self {
        Self {
            data,
            attempted: false,
            initial_id: None,
            initial_settled: false,
            initial_update: None,
            read_id: None,
            read_update: None,
            read_ids: vec![],
            record: None,
            revision: 0,
            snapshot: None,
            accepted_result: None,
            recovered: false,
        }
    }
    /// Persist this complete original before the caller's first possible send.
    pub(super) fn intent(&self) -> Value {
        json!({"format":"connect/physical-data-original/1","type":"message.send",
            "scope":{"conversation_id":self.data.conversation,"session_id":self.data.session,
                "connection_id":self.data.connection,"operation_id":self.data.operation},
            "payload":{"msg_id":self.data.message,"to":[self.data.recipient],
                "label":self.data.stream,"content_type":"application/octet-stream",
                "body_encoding":"base64","body":self.data.body,
                "reliability":{"mode":"reliable_ordered"}}})
    }
    /// Call only after the actual public completed-root check and durable save.
    /// Set custody before possible IO: even an unknown send can never re-enter.
    pub(super) fn begin(&mut self, original_root_completed: bool) -> Result<Value, &'static str> {
        if !original_root_completed || self.attempted {
            return Err("original Data mutation is not available for submission");
        }
        self.attempted = true;
        Ok(self.intent())
    }
    /// The original public send returned this wire correlation, not a new operation.
    pub(super) fn bind_submission(&mut self, request: &str) -> Result<(), &'static str> {
        if !self.attempted
            || self.initial_settled
            || self.initial_id.is_some()
            || !identifier(request)
        {
            return Err("original Data submission correlation changed");
        }
        self.initial_id = Some(request.into());
        Ok(())
    }
    pub(super) fn submission_unknown(&mut self) -> Result<(), &'static str> {
        if !self.attempted || self.initial_settled {
            return Err("original Data submission already settled");
        }
        self.initial_update = None;
        self.initial_settled = true;
        Ok(())
    }
    pub(super) fn submission_id(&self) -> Option<&str> {
        self.initial_id.as_deref()
    }
    pub(super) fn submission_pending(&self) -> bool {
        self.attempted && !self.initial_settled
    }
    pub(super) fn read_id(&self) -> Option<&str> {
        self.read_id.as_deref()
    }
    pub(super) fn can_read(&self) -> bool {
        self.initial_settled && self.read_id.is_none() && !self.accepted_core()
    }
    pub(super) fn read_unknown(&mut self) -> Result<(), &'static str> {
        if self.read_id.take().is_none() {
            return Err("no original Data read to abandon");
        }
        self.read_update = None;
        Ok(())
    }
    /// New wire IDs are permitted only for read-only observations of the same key.
    pub(super) fn bind_read(&mut self, request: &str) -> Result<(), &'static str> {
        if !self.initial_settled
            || self.read_id.is_some()
            || !identifier(request)
            || self.initial_id.as_deref() == Some(request)
            || self.read_ids.iter().any(|id| id == request)
            || self.read_ids.len() >= 256
        {
            return Err("original Data status-read correlation is unavailable");
        }
        self.read_ids.push(request.into());
        self.read_id = Some(request.into());
        Ok(())
    }
    pub(super) fn status_payload(&self) -> Value {
        json!({"operation_id":self.data.operation})
    }
    pub(super) fn accepted_core(&self) -> bool {
        self.accepted_result.is_some()
    }
    pub(super) fn recovered_read_only(&self) -> bool {
        self.recovered
    }
    fn scope(&self, frame: &Value, request: &str, mutation: bool) -> Result<(), &'static str> {
        let operation = frame.get("operation_id");
        if frame["v"] != 2
            || frame["cid"] != self.data.conversation
            || frame["sid"] != self.data.session
            || frame["connid"] != self.data.connection
            || frame["in_reply_to"] != request
            || frame.get("expected_revision").is_some()
            || if mutation {
                operation.and_then(Value::as_str) != Some(self.data.operation.as_str())
            } else {
                operation.is_some()
            }
        {
            return Err("original Data reply changed correlation or resource scope");
        }
        Ok(())
    }
    fn validate_update(&self, p: &Value) -> Result<(), &'static str> {
        let record = p["record_id"]
            .as_str()
            .filter(|id| identifier(id))
            .ok_or("original Data record missing")?;
        let revision = p["revision"]
            .as_u64()
            .filter(|r| *r > 0 && *r >= self.revision)
            .ok_or("original Data revision regressed")?;
        if p["operation_id"] != self.data.operation
            || p["command_type"] != "message.send"
            || !p["expected_revision"].is_null()
            || self.record.as_deref().is_some_and(|old| old != record)
        {
            return Err("original Data operation identity changed");
        }
        let state = p["state"].as_str().ok_or("original Data state missing")?;
        match state {
            "accepted" | "preparing" | "ready" | "committing" | "failed" | "cancelled"
            | "unknown" => {
                if self.accepted_result.is_some() {
                    return Err("completed original Data state changed");
                }
            }
            "completed" => {
                let result = &p["result"];
                if result["message_id"] != self.data.message
                    || result["connection_id"] != self.data.connection
                    || result["core_queue_admission"] != true
                    || !p["problem"].is_null()
                    || self
                        .accepted_result
                        .as_ref()
                        .is_some_and(|prior| prior != result)
                {
                    return Err("original Data acceptance changed its key or boundary");
                }
            }
            _ => return Err("unknown original Data state"),
        }
        if revision == self.revision
            && self.snapshot.as_ref().is_some_and(|old| {
                old["state"] != p["state"]
                    || old["result"] != p["result"]
                    || old["problem"] != p["problem"]
            })
        {
            return Err("same original Data revision changed its facts");
        }
        Ok(())
    }
    fn commit_update(&mut self, p: Value, read: bool) -> Result<Progress, &'static str> {
        self.validate_update(&p)?;
        self.record = Some(p["record_id"].as_str().unwrap().into());
        self.revision = p["revision"].as_u64().unwrap();
        let result = match p["state"].as_str().unwrap() {
            "completed" => {
                self.accepted_result = Some(p["result"].clone());
                if read {
                    self.recovered = true;
                }
                Progress::CoreAccepted
            }
            "failed" | "cancelled" | "unknown" => Progress::Refused,
            _ => Progress::Pending,
        };
        self.snapshot = Some(p);
        Ok(result)
    }
    pub(super) fn observe_submission(&mut self, frame: &Value) -> Result<Progress, &'static str> {
        let request = self
            .initial_id
            .as_deref()
            .ok_or("original Data submission has no wire correlation")?;
        if self.initial_settled {
            return Err("duplicate original Data submission reply");
        }
        self.scope(frame, request, true)?;
        match frame["type"].as_str() {
            Some("operation.updated") if self.initial_update.is_none() => {
                self.validate_update(&frame["payload"])?;
                self.initial_update = Some(frame["payload"].clone());
                Ok(Progress::Pending)
            }
            Some("ack") if frame["payload"].is_object() && self.initial_update.is_some() => {
                let update = self.initial_update.as_ref().unwrap().clone();
                let result = self.commit_update(update, false)?;
                self.initial_update = None;
                self.initial_settled = true;
                Ok(result)
            }
            Some("error")
                if self.initial_update.is_none()
                    && frame["payload"]["code"]
                        .as_u64()
                        .is_some_and(|c| (100..=599).contains(&c)) =>
            {
                self.initial_settled = true;
                Ok(Progress::Refused)
            }
            _ => Err("unexpected original Data submission reply"),
        }
    }
    pub(super) fn observe_read(&mut self, frame: &Value) -> Result<Progress, &'static str> {
        let request = self
            .read_id
            .as_deref()
            .ok_or("original Data status has no wire correlation")?;
        self.scope(frame, request, false)?;
        match frame["type"].as_str() {
            Some("operation.updated") if self.read_update.is_none() => {
                self.validate_update(&frame["payload"])?;
                self.read_update = Some(frame["payload"].clone());
                Ok(Progress::Pending)
            }
            Some("ack") if frame["payload"].is_object() && self.read_update.is_some() => {
                let update = self.read_update.as_ref().unwrap().clone();
                let result = self.commit_update(update, true)?;
                self.read_update = None;
                self.read_id = None;
                Ok(result)
            }
            Some("error")
                if self.read_update.is_none()
                    && frame["payload"]["code"]
                        .as_u64()
                        .is_some_and(|c| (100..=599).contains(&c)) =>
            {
                self.read_id = None;
                Ok(if frame["payload"]["code"] == 503 {
                    Progress::DeferredRead
                } else {
                    Progress::Refused
                })
            }
            _ => Err("unexpected original Data status reply"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn outgoing() -> Outgoing {
        Outgoing::new(
            ExpectedData::new(
                ["room", "session", "physical-target"],
                ["original-send", "original-message", "own-data"],
                ["local-participant", "remote-participant"],
                &STANDARD.encode(vec![7; 65536]),
            )
            .unwrap(),
        )
    }
    fn frame(kind: &str, request: &str, mutation: bool, payload: Value) -> Value {
        let mut f = json!({"v":2,"type":kind,"in_reply_to":request,"cid":"room","sid":"session","connid":"physical-target","payload":payload});
        if mutation {
            f["operation_id"] = json!("original-send");
        }
        f
    }
    fn update(state: &str, revision: u64) -> Value {
        let mut p = json!({"record_id":"original-record","operation_id":"original-send","command_type":"message.send","revision":revision,"state":state});
        if state == "completed" {
            p["result"] = json!({"message_id":"original-message","connection_id":"physical-target","core_queue_admission":true});
        }
        p
    }
    fn start(o: &mut Outgoing) {
        o.begin(true).unwrap();
        o.bind_submission("wire-send").unwrap();
    }
    fn settle(o: &mut Outgoing, state: &str, revision: u64) -> Progress {
        assert_eq!(
            o.observe_submission(&frame(
                "operation.updated",
                "wire-send",
                true,
                update(state, revision)
            ))
            .unwrap(),
            Progress::Pending
        );
        o.observe_submission(&frame("ack", "wire-send", true, json!({})))
            .unwrap()
    }
    #[test]
    fn unknown_submission_never_reenters_or_changes_original_intent() {
        let mut o = outgoing();
        let intent = o.intent();
        assert!(o.begin(false).is_err());
        assert_eq!(o.begin(true).unwrap(), intent);
        assert!(o.begin(true).is_err());
        assert_eq!(o.intent(), intent);
        assert!(!o.accepted_core());
    }
    #[test]
    fn unknown_send_without_wire_id_recovers_original_by_read_only_and_never_rebinds_mutation() {
        let mut o = outgoing();
        let original = o.begin(true).unwrap();
        o.submission_unknown().unwrap();
        assert!(o.bind_submission("late-send-wire").is_err());
        assert!(o.begin(true).is_err());
        assert!(o.can_read());
        o.bind_read("original-read").unwrap();
        o.observe_read(&frame(
            "operation.updated",
            "original-read",
            false,
            update("completed", 4),
        ))
        .unwrap();
        assert_eq!(
            o.observe_read(&frame("ack", "original-read", false, json!({})))
                .unwrap(),
            Progress::CoreAccepted
        );
        assert!(o.accepted_core() && o.recovered_read_only());
        assert_eq!(o.intent(), original);
    }
    #[test]
    fn missing_read_response_keeps_original_key_and_refuses_late_or_reused_correlations() {
        let mut o = outgoing();
        start(&mut o);
        o.submission_unknown().unwrap();
        o.bind_read("read-one").unwrap();
        o.read_unknown().unwrap();
        assert!(o.bind_read("read-one").is_err());
        assert!(o.read_unknown().is_err());
        assert!(o
            .observe_read(&frame("ack", "read-one", false, json!({})))
            .is_err());
        o.bind_read("read-two").unwrap();
        assert!(o
            .observe_read(&frame("ack", "read-one", false, json!({})))
            .is_err());
        assert!(!o.accepted_core());
        assert!(o.begin(true).is_err());
    }
    #[test]
    fn correlated_completed_update_needs_its_ack_and_is_only_core_acceptance() {
        let mut o = outgoing();
        start(&mut o);
        o.observe_submission(&frame(
            "operation.updated",
            "wire-send",
            true,
            update("completed", 3),
        ))
        .unwrap();
        assert!(!o.accepted_core());
        assert!(o.bind_read("too-early").is_err());
        assert_eq!(
            o.observe_submission(&frame("ack", "wire-send", true, json!({})))
                .unwrap(),
            Progress::CoreAccepted
        );
        assert!(o.accepted_core());
        assert!(!o.recovered_read_only());
        assert!(o
            .observe_submission(&frame("ack", "wire-send", true, json!({})))
            .is_err());
    }
    #[test]
    fn pending_and_503_status_reads_recover_only_the_original_operation() {
        let mut o = outgoing();
        let intent = o.intent();
        start(&mut o);
        assert_eq!(settle(&mut o, "preparing", 2), Progress::Pending);
        o.bind_read("read-one").unwrap();
        assert_eq!(
            o.observe_read(&frame("error", "read-one", false, json!({"code":503})))
                .unwrap(),
            Progress::DeferredRead
        );
        assert!(!o.accepted_core());
        assert!(o.begin(true).is_err());
        assert!(o.bind_read("read-one").is_err());
        o.bind_read("read-two").unwrap();
        assert_eq!(o.status_payload(), json!({"operation_id":"original-send"}));
        o.observe_read(&frame(
            "operation.updated",
            "read-two",
            false,
            update("completed", 4),
        ))
        .unwrap();
        assert!(!o.accepted_core());
        assert_eq!(
            o.observe_read(&frame("ack", "read-two", false, json!({})))
                .unwrap(),
            Progress::CoreAccepted
        );
        assert!(o.accepted_core() && o.recovered_read_only());
        assert_eq!(o.intent(), intent);
        assert!(o.begin(true).is_err());
    }
    #[test]
    fn refusal_preserves_original_key_without_creating_a_second_mutation() {
        let mut o = outgoing();
        start(&mut o);
        assert_eq!(
            o.observe_submission(&frame("error", "wire-send", true, json!({"code":503})))
                .unwrap(),
            Progress::Refused
        );
        assert!(!o.accepted_core());
        assert!(o.begin(true).is_err());
        o.bind_read("original-read").unwrap();
        assert_eq!(o.status_payload(), json!({"operation_id":"original-send"}));
    }
    #[test]
    fn foreign_scope_hint_key_message_or_boundary_cannot_lend_acceptance() {
        for key in ["cid", "sid", "connid", "operation_id", "in_reply_to"] {
            let mut o = outgoing();
            start(&mut o);
            let mut f = frame(
                "operation.updated",
                "wire-send",
                true,
                update("completed", 3),
            );
            f[key] = json!("foreign");
            assert!(o.observe_submission(&f).is_err());
            assert!(!o.accepted_core());
        }
        for key in ["operation_id", "command_type", "revision"] {
            let mut o = outgoing();
            start(&mut o);
            let mut p = update("completed", 3);
            p[key] = json!("foreign");
            assert!(o
                .observe_submission(&frame("operation.updated", "wire-send", true, p))
                .is_err());
            assert!(!o.accepted_core());
        }
        for key in ["message_id", "connection_id", "core_queue_admission"] {
            let mut o = outgoing();
            start(&mut o);
            let mut p = update("completed", 3);
            p["result"][key] = json!("foreign");
            assert!(o
                .observe_submission(&frame("operation.updated", "wire-send", true, p))
                .is_err());
            assert!(!o.accepted_core());
        }
    }
    #[test]
    fn first_server_record_is_bound_once_and_invalid_record_shapes_are_refused() {
        let mut o = outgoing();
        start(&mut o);
        let mut p = update("completed", 3);
        p["record_id"] = json!("server-chosen-record");
        o.observe_submission(&frame("operation.updated", "wire-send", true, p.clone()))
            .unwrap();
        o.observe_submission(&frame("ack", "wire-send", true, json!({})))
            .unwrap();
        assert!(o.accepted_core());
        o.bind_read("same-record-read").unwrap();
        o.observe_read(&frame(
            "operation.updated",
            "same-record-read",
            false,
            p.clone(),
        ))
        .unwrap();
        o.observe_read(&frame("ack", "same-record-read", false, json!({})))
            .unwrap();
        assert!(o.recovered_read_only());
        for invalid in [Value::Null, json!(""), json!("invalid record identity")] {
            let mut o = outgoing();
            start(&mut o);
            let mut p = update("completed", 3);
            p["record_id"] = invalid;
            assert!(o
                .observe_submission(&frame("operation.updated", "wire-send", true, p))
                .is_err());
            assert!(!o.accepted_core());
        }
    }
    #[test]
    fn recovery_refuses_changed_record_regressed_revision_or_different_completed_result() {
        for kind in ["record", "revision", "result", "state"] {
            let mut o = outgoing();
            start(&mut o);
            assert_eq!(settle(&mut o, "completed", 3), Progress::CoreAccepted);
            o.bind_read("recover").unwrap();
            let mut p = update("completed", 4);
            match kind {
                "record" => p["record_id"] = json!("other-record"),
                "revision" => p["revision"] = json!(2),
                "result" => p["result"]["extra"] = json!(true),
                _ => p["state"] = json!("unknown"),
            };
            assert!(o
                .observe_read(&frame("operation.updated", "recover", false, p))
                .is_err());
            assert!(!o.recovered_read_only());
        }
    }
    #[test]
    fn completed_original_readback_requires_exact_unchanged_result_and_own_ack() {
        let mut o = outgoing();
        start(&mut o);
        settle(&mut o, "completed", 3);
        o.bind_read("recover").unwrap();
        assert!(o.bind_read("parallel-read").is_err());
        o.observe_read(&frame(
            "operation.updated",
            "recover",
            false,
            update("completed", 3),
        ))
        .unwrap();
        assert!(!o.recovered_read_only());
        assert!(o
            .observe_read(&frame("ack", "foreign", false, json!({})))
            .is_err());
        assert_eq!(
            o.observe_read(&frame("ack", "recover", false, json!({})))
                .unwrap(),
            Progress::CoreAccepted
        );
        assert!(o.recovered_read_only());
        assert!(o.bind_submission("other-mutation-wire").is_err());
    }
}
