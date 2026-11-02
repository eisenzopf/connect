//! Match signed negotiated resources to the complete original offer.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Binding {
    pub id: String,
    pub handle: u16,
    pub pt: Option<u8>,
    pub raw: Value,
}
pub(super) fn binding(raw: &Value, id: &str, kind: &str, direction: &str) -> Result<Binding> {
    let (codec, clock) = match kind {
        "audio" => ("opus", Some((48000, 1))),
        "video" => ("vp8", Some((90000, 0))),
        "data" => ("binary", None),
        _ => return Err("unsupported replacement medium".into()),
    };
    let handle = raw["stream_local_id"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
        .filter(|v| *v > 0)
        .ok_or("invalid replacement hop handle")?;
    let parameters = &raw["codec"]["params"];
    let pt = if let Some((rate, channels)) = clock {
        if parameters["sample_rate"] != rate || parameters["channels"] != channels {
            return Err("replacement media clock differs from the original profile".into());
        }
        Some(
            parameters["payload_type"]
                .as_u64()
                .and_then(|v| u8::try_from(v).ok())
                .filter(|v| *v < 128)
                .ok_or("invalid replacement payload type")?,
        )
    } else {
        if parameters != &json!({"reliability":"reliable_ordered"}) {
            return Err("replacement Data must retain its reliable ordered binary format".into());
        }
        None
    };
    if raw["strm_id"] != id
        || raw["kind"] != kind
        || raw["direction"] != direction
        || raw["codec"]["name"] != codec
    {
        return Err("replacement negotiated resources differ from the original profile".into());
    }
    Ok(Binding {
        id: id.into(),
        handle,
        pt,
        raw: raw.clone(),
    })
}
pub(super) fn own(payload: &Value, offered: &Value) -> Result<Vec<Binding>> {
    let expected = offered["streams_offered"]
        .as_array()
        .ok_or("original replacement offer required")?;
    let actual = payload["substrate_setup"]["streams"]
        .as_array()
        .ok_or("replacement negotiation omitted resources")?;
    if actual.len() != expected.len() || !(1..=3).contains(&actual.len()) {
        return Err("replacement resource count changed".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut handles = std::collections::BTreeSet::new();
    let mut kinds = std::collections::BTreeSet::new();
    let mut result = vec![];
    for stream in expected {
        let id = stream["id"]
            .as_str()
            .ok_or("missing original stream identity")?;
        if !ids.insert(id) {
            return Err("duplicate original stream identity".into());
        }
        let found = actual
            .iter()
            .filter(|v| v["strm_id"] == id)
            .collect::<Vec<_>>();
        if found.len() != 1 {
            return Err("replacement negotiation changed original stream identity".into());
        }
        let kind = stream["kind"].as_str().ok_or("missing original medium")?;
        if !kinds.insert(kind) {
            return Err("replacement offered more than one stream of the same medium".into());
        }
        let item = binding(found[0], id, kind, "sendrecv")?;
        if !handles.insert(item.handle) {
            return Err("replacement hop handle reused across streams".into());
        }
        result.push(item);
    }
    Ok(result)
}
pub(super) fn subscription(payload: &Value, own: &[Binding], remote: &str) -> Result<Binding> {
    let streams = payload["substrate_setup"]["streams"]
        .as_array()
        .ok_or("missing subscription parameters")?;
    if streams.len() != own.len() + 1 {
        return Err("unexpected replacement subscription resources".into());
    }
    for item in own {
        let found = streams
            .iter()
            .filter(|v| v["strm_id"] == item.id)
            .collect::<Vec<_>>();
        if found.len() != 1 || *found[0] != item.raw {
            return Err("subscription changed original replacement resources".into());
        }
    }
    let extra = streams
        .iter()
        .filter(|v| v["strm_id"] == remote)
        .collect::<Vec<_>>();
    if extra.len() != 1 {
        return Err("original retained video resource missing".into());
    }
    let binding = binding(extra[0], remote, "video", "sendonly")?;
    if own.iter().any(|v| v.handle == binding.handle) {
        return Err("incoming video reused an original hop handle".into());
    }
    Ok(binding)
}
pub(super) fn answer(payload: &Value, capabilities: &Value) -> Result<payloads::ConnectionAnswer> {
    let streams=payload["substrate_setup"]["streams"].as_array().ok_or("missing native parameters")?.iter().map(|s|json!({
        "id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str(){Some("sendonly")=>"recvonly",Some("recvonly")=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
    Ok(serde_json::from_value(
        json!({"by_participant":payload["by_participant"],"substrate":"quic","capabilities":capabilities,"streams_answered":streams,"substrate_setup":payload["substrate_setup"]}),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reliable_data_is_preserved_without_inventing_rtp_fields_or_rebinding_its_handle() {
        let data = json!({"strm_id":"original-data","kind":"data","direction":"sendrecv",
            "stream_local_id":10,"codec":{"name":"binary","params":{"reliability":"reliable_ordered"}}});
        let audio = json!({"strm_id":"audio","kind":"audio","direction":"sendrecv","stream_local_id":7,
            "codec":{"name":"opus","params":{"sample_rate":48000,"channels":1,"payload_type":111}}});
        let camera = json!({"strm_id":"camera","kind":"video","direction":"sendrecv","stream_local_id":8,
            "codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}}});
        let offer = json!({"streams_offered":[{"id":"audio","kind":"audio"},
            {"id":"camera","kind":"video"},{"id":"original-data","kind":"data"}]});
        let good =
            json!({"substrate_setup":{"streams":[data.clone(),camera.clone(),audio.clone()]}});
        let own = own(&good, &offer).unwrap();
        assert_eq!(own[2].id, "original-data");
        assert_eq!(own[2].handle, 10);
        assert_eq!(own[2].pt, None);
        for field in ["payload_type", "sample_rate", "channels"] {
            let mut wrong = good.clone();
            wrong["substrate_setup"]["streams"][0]["codec"]["params"][field] = json!(1);
            assert!(super::own(&wrong, &offer).is_err());
        }
        for (field, value) in [
            ("stream_local_id", json!(7)),
            ("strm_id", json!("other-data")),
            ("direction", json!("recvonly")),
            (
                "codec",
                json!({"name":"binary","params":{"reliability":"unordered"}}),
            ),
        ] {
            let mut wrong = good.clone();
            wrong["substrate_setup"]["streams"][0][field] = value;
            assert!(super::own(&wrong, &offer).is_err());
        }
        let mut missing = good.clone();
        missing["substrate_setup"]["streams"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        assert!(super::own(&missing, &offer).is_err());
        let mut incoming = camera;
        incoming["strm_id"] = json!("remote-camera");
        incoming["direction"] = json!("sendonly");
        incoming["stream_local_id"] = json!(11);
        let mut subscribed = good;
        subscribed["substrate_setup"]["streams"]
            .as_array_mut()
            .unwrap()
            .push(incoming);
        assert_eq!(
            subscription(&subscribed, &own, "remote-camera")
                .unwrap()
                .handle,
            11
        );
        subscribed["substrate_setup"]["streams"][0]["stream_local_id"] = json!(12);
        assert!(subscription(&subscribed, &own, "remote-camera").is_err());
    }
    #[test]
    fn signed_av_resources_are_matched_by_original_ids_not_array_position_or_payload_type_guessing()
    {
        let audio = json!({"strm_id":"audio","kind":"audio","direction":"sendrecv","stream_local_id":7,
            "codec":{"name":"opus","params":{"sample_rate":48000,"channels":1,"payload_type":111}}});
        let video = json!({"strm_id":"camera","kind":"video","direction":"sendrecv","stream_local_id":8,
            "codec":{"name":"vp8","params":{"sample_rate":90000,"channels":0,"payload_type":96}}});
        let offer = json!({"streams_offered":[{"id":"audio","kind":"audio"},{"id":"camera","kind":"video"}]});
        let good = json!({"substrate_setup":{"streams":[video.clone(),audio.clone()]}});
        let selected = own(&good, &offer).unwrap();
        assert_eq!(selected[0].id, "audio");
        assert_eq!(selected[1].id, "camera");
        for change in 0..4 {
            let mut wrong = good.clone();
            match change {
                0 => wrong["substrate_setup"]["streams"][0]["stream_local_id"] = json!(7),
                1 => wrong["substrate_setup"]["streams"][0]["strm_id"] = json!("other"),
                2 => wrong["substrate_setup"]["streams"][0]["direction"] = json!("sendonly"),
                _ => wrong["substrate_setup"]["streams"]
                    .as_array_mut()
                    .unwrap()
                    .push(video.clone()),
            }
            assert!(own(&wrong, &offer).is_err(), "{change}");
        }
        let mut incoming = video.clone();
        incoming["strm_id"] = json!("remote-camera");
        incoming["stream_local_id"] = json!(9);
        incoming["direction"] = json!("sendonly");
        let subscribed = json!({"substrate_setup":{"streams":[audio,video,incoming]}});
        assert_eq!(
            subscription(&subscribed, &selected, "remote-camera")
                .unwrap()
                .handle,
            9
        );
        let mut wrong = subscribed;
        wrong["substrate_setup"]["streams"][0]["stream_local_id"] = json!(12);
        assert!(subscription(&wrong, &selected, "remote-camera").is_err());
    }
}
