//! The actual signed AV helper's operation-state gate, not a protocol mock.
use super::*;

pub(super) struct Completion {
    operation: String,
    record: Option<String>,
    last: Option<Value>,
}
impl Completion {
    pub(super) fn new(operation: &str) -> Self {
        Self {
            operation: operation.into(),
            record: None,
            last: None,
        }
    }
    pub(super) fn observe(&mut self, frame: &Envelope, reply: &str) {
        assert_eq!(frame.kind(), "operation.updated");
        assert_eq!(
            frame.original()["in_reply_to"],
            reply,
            "signed AV observation belongs to another wire request"
        );
        assert_eq!(
            frame.scope(connect_client::canonical::Scope::Operation),
            Some(self.operation.as_str())
        );
        let value = frame.original()["payload"].clone();
        assert_eq!(
            value["operation_id"].as_str(),
            Some(self.operation.as_str()),
            "signed AV read observed another logical operation"
        );
        let state = value["state"].as_str().expect("signed AV state missing");
        assert!(
            matches!(
                state,
                "accepted" | "preparing" | "ready" | "committing" | "completed"
            ),
            "signed AV operation entered terminal or invalid state: {value:?}"
        );
        let record = value["record_id"].as_str();
        if let Some(original) = &self.record {
            assert_eq!(
                record,
                Some(original.as_str()),
                "signed AV record identity changed"
            );
        }
        if let Some(previous) = &self.last {
            assert!(
                value["revision"]
                    .as_u64()
                    .expect("signed AV revision missing")
                    >= previous["revision"]
                        .as_u64()
                        .expect("previous signed AV revision missing"),
                "signed AV observation revision regressed"
            );
            if previous["state"] == "completed" {
                assert_eq!(&value, previous, "signed AV completed result changed");
            }
        } else {
            assert!(
                value["revision"].as_u64().is_some(),
                "signed AV revision missing"
            );
        }
        if self.record.is_none() {
            self.record = record.map(str::to_owned);
        }
        self.last = Some(value);
    }
    pub(super) fn completed(&self) -> Option<Value> {
        self.last
            .as_ref()
            .filter(|value| value["state"] == "completed")
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(value: Value, reply: &str) -> Envelope {
        Envelope::from_value(json!({"v":2,"type":"operation.updated",
            "id":format!("fact-{}",value["revision"]),"ts":chrono::Utc::now(),
            "cid":"room","sid":"native-session","connid":"controller",
            "operation_id":value["operation_id"],"in_reply_to":reply,
            "payload":value}))
        .unwrap()
    }
    fn update(state: &str, revision: u64) -> Value {
        let mut value = json!({"operation_id":"original-av-subscription", "record_id":"original-record",
            "command_type":"stream.subscribe", "expected_revision":7,
            "state":state,"revision":revision});
        if state == "completed" {
            value["result"] = json!({"stream_revision":8});
        }
        value
    }
    #[test]
    fn actual_av_completion_accepts_progress_but_returns_only_original_completed_result() {
        let mut state = Completion::new("original-av-subscription");
        for (revision, phase) in ["accepted", "preparing", "ready", "committing"]
            .into_iter()
            .enumerate()
        {
            let reply = if revision == 0 {
                "original-mutation".into()
            } else {
                format!("read-{revision}")
            };
            state.observe(&frame(update(phase, revision as u64 + 1), &reply), &reply);
            assert!(
                state.completed().is_none(),
                "progress/ACK is not completion"
            );
        }
        let completed = update("completed", 5);
        state.observe(&frame(completed.clone(), "read-final"), "read-final");
        assert_eq!(state.completed(), Some(completed));
    }
    #[test]
    fn actual_av_completion_refuses_failed_unknown_and_cancelled_outcomes() {
        for phase in ["failed", "cancelled", "interrupted", "unknown"] {
            let mut state = Completion::new("original-av-subscription");
            state.observe(
                &frame(update("preparing", 1), "original-mutation"),
                "original-mutation",
            );
            let terminal = frame(update(phase, 2), "read-next");
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || state.observe(&terminal, "read-next")
            ))
            .is_err());
            assert!(state.completed().is_none());
        }
    }
    #[test]
    fn actual_av_completion_refuses_foreign_record_request_regression_and_changed_final_result() {
        for mode in 0..5 {
            let mut state = Completion::new("original-av-subscription");
            state.observe(
                &frame(update("preparing", 3), "original-mutation"),
                "original-mutation",
            );
            let mut invalid = update("completed", 4);
            let mut reply = "read-next";
            match mode {
                0 => invalid["operation_id"] = json!("foreign-operation"),
                1 => invalid["record_id"] = json!("foreign-record"),
                2 => invalid["revision"] = json!(2),
                3 => reply = "foreign-wire-request",
                _ => {
                    state.observe(&frame(update("completed", 4), "read-prior"), "read-prior");
                    invalid["result"] = json!({"stream_revision":999});
                }
            }
            let invalid = frame(invalid, reply);
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || state.observe(&invalid, "read-next")
            ))
            .is_err());
        }
    }
}
