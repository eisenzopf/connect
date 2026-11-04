use super::*;

fn ready() -> Shared {
    let shared = Shared::default();
    stage(&shared, "offer");
    shared.confirmed().unwrap();
    for publisher in [false, true] {
        shared.verified(&epoch("offer", 1, publisher)).unwrap();
    }
    shared
}
fn command(operation: &str, expected: u64, action: &str) -> Value {
    let mut v = scoped(
        "connection.update",
        json!({"action":action,"streams":["camera"]}),
        None,
    )
    .original()
    .clone();
    v["operation_id"] = json!(operation);
    v["expected_revision"] = json!(expected);
    if action == "hold" {
        v["payload"]["details"] = json!({"direction":"inactive"});
    }
    v
}
fn completed(command: &Value) -> Value {
    let action = command["payload"]["action"].as_str().unwrap();
    let codec = pair("offer").0["substrate_setup"]["streams"][0]["codec"].clone();
    let gate = json!({"binding":{"stream_id":"camera","medium":"video","direction":"sendrecv","codec":codec,"stream_local_id":7,"stream_generation":1,"connection_generation":1},"state":if action=="hold" {"paused"}else{"active"},"source_muted":action=="mute","held_direction":null});
    let mut v = scoped("operation.updated", json!({"operation_id":command["operation_id"],"command_type":"connection.update","expected_revision":command["expected_revision"],"state":"completed","revision":2,"result":{"connection_id":"connection","connection_generation":1,"stream_revision":command["expected_revision"].as_u64().unwrap()+1,"resources_changed":false,"action":action,"gate_streams":[gate]}}), Some(command["id"].as_str().unwrap())).original().clone();
    v["operation_id"] = command["operation_id"].clone();
    v
}
fn deliver(shared: &Shared, value: Value) -> Result<bool> {
    shared.verified(&Envelope::from_value(value).unwrap())
}
fn active(shared: &Shared, publisher: bool) -> Option<u64> {
    shared
        .state
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .active
        .get(&(7, publisher))
        .map(|n| n.history.revision)
}

#[test]
fn replacement_video_gate_results_require_fresh_history_and_preserve_other_direction() {
    let shared = ready();
    let mute = command("mute", 4, "mute");
    shared.outgoing(&mute).unwrap();
    assert_eq!(active(&shared, false), Some(1));
    assert_eq!(active(&shared, true), None);
    deliver(&shared, completed(&mute)).unwrap();
    shared.verified(&epoch("offer", 2, true)).unwrap();
    assert_eq!(active(&shared, true), None);
    let unmute = command("unmute", 5, "unmute");
    shared.outgoing(&unmute).unwrap();
    // Neither the old epoch nor a receipt alone revives the publisher.
    deliver(&shared, completed(&unmute)).unwrap();
    shared.verified(&epoch("offer", 2, true)).unwrap();
    assert_eq!(active(&shared, true), None);
    shared.verified(&epoch("offer", 3, true)).unwrap();
    assert_eq!(active(&shared, true), Some(3));
    assert_eq!(active(&shared, false), Some(1));
    // Same logical replay preserves current history. Changed reuse is refused.
    shared.outgoing(&unmute).unwrap();
    assert_eq!(active(&shared, true), Some(3));
    let mut changed = unmute.clone();
    changed["payload"]["action"] = json!("mute");
    assert!(shared.outgoing(&changed).is_err());
    deliver(&shared, completed(&mute)).unwrap();
    assert_eq!(active(&shared, true), Some(3));
}

#[test]
fn replacement_video_gate_resume_joins_early_notice_and_cannot_restore_unsubscribed_history() {
    let shared = ready();
    let hold = command("hold", 4, "hold");
    shared.outgoing(&hold).unwrap();
    deliver(&shared, completed(&hold)).unwrap();
    let resume = command("resume", 5, "resume");
    shared.outgoing(&resume).unwrap();
    for publisher in [false, true] {
        shared.verified(&epoch("offer", 2, publisher)).unwrap();
    }
    assert_eq!(active(&shared, true), None);
    assert_eq!(active(&shared, false), None);
    deliver(&shared, completed(&resume)).unwrap();
    assert_eq!(active(&shared, true), Some(2));
    assert_eq!(active(&shared, false), Some(2));
    shared
        .outgoing(scoped("stream.unsubscribe", json!({"strm_ids":["camera"]}), None).original())
        .unwrap();
    let mute = command("mute", 6, "mute");
    shared.outgoing(&mute).unwrap();
    deliver(&shared, completed(&mute)).unwrap();
    assert_eq!(active(&shared, false), None);
    // Retirement wins over a completed old operation and a new signed notice.
    shared
        .outgoing(
            scoped(
                "connection.end",
                json!({"reason_code":200,"reason":"done"}),
                None,
            )
            .original(),
        )
        .unwrap();
    shared.verified(&epoch("offer", 3, true)).unwrap();
    deliver(&shared, completed(&resume)).unwrap();
    assert_eq!(active(&shared, true), None);
}

#[test]
fn replacement_video_gate_results_refuse_foreign_binding_revision_and_uncertain_completion() {
    for field in [
        "stream_local_id",
        "stream_generation",
        "connection_generation",
        "codec",
        "direction",
    ] {
        let shared = ready();
        let hold = command("hold", 4, "hold");
        shared.outgoing(&hold).unwrap();
        let mut receipt = completed(&hold);
        receipt["payload"]["result"]["gate_streams"][0]["binding"][field] = match field {
            "codec" => json!({"name":"h264"}),
            "direction" => json!("sendonly"),
            _ => json!(99),
        };
        assert!(deliver(&shared, receipt).is_err(), "{field}");
        assert_eq!(active(&shared, true), None);
    }
    let shared = ready();
    let hold = command("hold", 4, "hold");
    shared.outgoing(&hold).unwrap();
    let mut foreign = completed(&hold);
    foreign["connid"] = json!("other");
    deliver(&shared, foreign).unwrap();
    let mut stale = completed(&hold);
    stale["payload"]["result"]["stream_revision"] = json!(4);
    assert!(deliver(&shared, stale).is_err());
    let mut uncertain = completed(&hold);
    uncertain["payload"]["state"] = json!("unknown");
    uncertain["payload"]
        .as_object_mut()
        .unwrap()
        .remove("result");
    deliver(&shared, uncertain).unwrap();
    shared.verified(&epoch("offer", 2, true)).unwrap();
    assert_eq!(active(&shared, true), None);
    // Recovery reads may have a new request id; the original logical operation
    // and complete immutable intent still have to match.
    let mut recovered = completed(&hold);
    recovered["in_reply_to"] = json!("original-operation-status-read");
    recovered.as_object_mut().unwrap().remove("operation_id");
    deliver(&shared, recovered).unwrap();
    assert_eq!(active(&shared, true), None);
    let mut wrong = command("new", 3, "resume");
    assert!(shared.outgoing(&wrong).is_err());
    wrong["expected_revision"] = json!(5);
    shared.outgoing(&wrong).unwrap();
    let mut failed = completed(&wrong);
    failed["payload"]["state"] = json!("failed");
    failed["payload"].as_object_mut().unwrap().remove("result");
    deliver(&shared, failed).unwrap();
    assert_eq!(active(&shared, true), None);
}

#[test]
fn replacement_video_explicit_data_gate_preserves_video_and_checks_sealed_data_receipt() {
    for generation in [1, 2, 3] {
        let shared = Shared::default();
        let mut offer = original();
        offer["capabilities"]["data_protocols"] = json!(["json"]);
        let data_offer = json!({"id":"messages","kind":"data","direction":"sendrecv","codec_preferences":["json"]});
        offer["streams_offered"]
            .as_array_mut()
            .unwrap()
            .push(data_offer.clone());
        shared
            .begin("offer".into(), scope(), offer.clone())
            .unwrap();
        let (mut answer, mut callback) = pair("offer");
        let codec = json!({"name":"json","params":{}});
        for payload in [&mut answer, &mut callback] {
            payload["capabilities"] = offer["capabilities"].clone();
            payload["substrate_setup"]["streams"][0]["video_feedback"]["connection_generation"] =
                json!(generation);
            payload["substrate_setup"]["streams"].as_array_mut().unwrap().push(json!({"strm_id":"messages","kind":"data","direction":"sendrecv","stream_local_id":8,"codec":codec}));
        }
        answer["streams_answered"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"messages","kind":"data","direction":"sendrecv","codec":codec}));
        callback["streams_offered"]
            .as_array_mut()
            .unwrap()
            .push(data_offer);
        shared
            .verified(&scoped("connection.answer", answer, Some("offer")))
            .unwrap();
        shared
            .verified(&scoped("connection.offer", callback, None))
            .unwrap();
        shared.confirmed().unwrap();
        for publisher in [false, true] {
            let mut value = epoch("offer", 1, publisher).original().clone();
            value["payload"]["connection_generation"] = json!(generation);
            deliver(&shared, value).unwrap();
        }
        let mut data = command("data-unmute", 4, "unmute");
        data["payload"]["streams"] = json!(["messages"]);
        // The exact original Data intent is accepted without retiring either
        // active video direction. A foreign target still refuses locally.
        let mut foreign = data.clone();
        foreign["payload"]["streams"] = json!(["foreign"]);
        assert!(shared.outgoing(&foreign).is_err());
        shared.outgoing(&data).unwrap();
        for publisher in [false, true] {
            assert_eq!(active(&shared, publisher), Some(1));
        }
        let mut receipt = completed(&data);
        receipt["payload"]["result"]["connection_generation"] = json!(generation);
        receipt["payload"]["result"]["gate_streams"] = json!([{"binding":{"stream_id":"messages","medium":"data","direction":"sendrecv","codec":codec,"stream_local_id":8,"stream_generation":1,"connection_generation":generation},"state":"active","source_muted":false,"held_direction":null}]);
        for field in [
            "stream_local_id",
            "stream_generation",
            "connection_generation",
        ] {
            let mut wrong = receipt.clone();
            wrong["payload"]["result"]["gate_streams"][0]["binding"][field] = json!(99);
            assert!(deliver(&shared, wrong).is_err());
        }
        deliver(&shared, receipt.clone()).unwrap();
        shared.outgoing(&data).unwrap();
        deliver(&shared, receipt).unwrap();
        for publisher in [false, true] {
            assert_eq!(active(&shared, publisher), Some(1));
        }
        assert_eq!(
            shared
                .state
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .selected
                .as_ref()
                .unwrap()
                .stream_revision
                .get(),
            5
        );
        let mut changed = data;
        changed["payload"]["action"] = json!("mute");
        assert!(shared.outgoing(&changed).is_err());
    }
}
