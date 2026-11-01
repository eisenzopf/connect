//! Match signed negotiated resources to the complete original offer.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Binding {
    pub id: String,
    pub handle: u16,
    pub pt: u8,
    pub raw: Value,
}
pub(super) fn binding(raw: &Value, id: &str, kind: &str, direction: &str) -> Result<Binding> {
    let (codec, rate, channels) = match kind {
        "audio" => ("opus", 48000, 1),
        "video" => ("vp8", 90000, 0),
        _ => return Err("unsupported replacement medium".into()),
    };
    let handle = raw["stream_local_id"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
        .filter(|v| *v > 0)
        .ok_or("invalid replacement hop handle")?;
    let pt = raw["codec"]["params"]["payload_type"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
        .filter(|v| *v < 128)
        .ok_or("invalid replacement payload type")?;
    if raw["strm_id"] != id
        || raw["kind"] != kind
        || raw["direction"] != direction
        || raw["codec"]["name"] != codec
        || raw["codec"]["params"]["sample_rate"] != rate
        || raw["codec"]["params"]["channels"] != channels
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
    if actual.len() != expected.len() || !(1..=2).contains(&actual.len()) {
        return Err("replacement resource count changed".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut handles = std::collections::BTreeSet::new();
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
        let item = binding(
            found[0],
            id,
            stream["kind"].as_str().ok_or("missing original medium")?,
            "sendrecv",
        )?;
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
