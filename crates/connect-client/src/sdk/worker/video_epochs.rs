//! Consume signed lifecycle metadata on the existing physical peer owner.
use super::super::video_feedback::epochs::Notice;
use super::*;

pub(super) fn frame(
    cache: &std::sync::Mutex<NativeCache>,
    media: &media::Inbox,
    frame: &crate::canonical::Envelope,
) -> Result<bool> {
    if frame.kind() != "stream.epoch" {
        return Ok(false);
    }
    let notice = Notice::read(frame)?;
    let mut cache = cache
        .lock()
        .map_err(|_| Error::local(ErrorKind::Protocol))?;
    let Some(entry) = cache.get_mut(&notice.key.connection_id).filter(|e| {
        !e.ended
            && e.conversation == notice.key.conversation_id
            && e.session == notice.key.session_id
    }) else {
        return Ok(true);
    };
    if notice.matches(&entry.selected) {
        media.install_epoch(notice)?;
    } else if let Some(prepared) = entry
        .prepared
        .as_mut()
        .filter(|p| notice.matches(&p.selected))
    {
        // Confirmation can precede the paired durable operation capture. Keep
        // bounded metadata only; preparation does not admit media or feedback.
        let key = (
            notice.key.stream_id.clone(),
            notice.role == super::super::video_feedback::epochs::Role::Publisher,
        );
        if prepared
            .video_epochs
            .get(&key)
            .is_none_or(|old| old.history.revision < notice.history.revision)
        {
            if prepared.video_epochs.len() >= 128 && !prepared.video_epochs.contains_key(&key) {
                return Err(Error::local(ErrorKind::Protocol));
            }
            prepared.video_epochs.insert(key, notice);
        }
    }
    // Retired or otherwise unmatched allocation metadata never enters the
    // generic unmatched-event queue and never changes decoder history.
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::super::video_feedback::epochs::tests::{notice_value, selected};
    use super::*;
    use serde_json::json;

    #[tokio::test(start_paused = true)]
    async fn signed_notice_waits_for_paired_resource_adoption_on_its_own_physical_client() {
        let selected = selected();
        let stream = selected.streams[0].clone();
        let key = OperationKey {
            operation_id: "operation".parse().unwrap(),
            command: "stream.subscribe".into(),
            scope: OperationScope {
                conversation_id: Some("room".parse().unwrap()),
                session_id: Some("session".parse().unwrap()),
                connection_id: Some("connection".parse().unwrap()),
            },
        };
        let empty = NativeNegotiation {
            streams: vec![],
            video_feedback: Default::default(),
            ..selected.clone()
        };
        let entry = NativeConnection {
            session: key.scope.session_id.clone().unwrap(),
            conversation: key.scope.conversation_id.clone().unwrap(),
            selected: empty,
            ended: false,
            known_revision: StreamRevision::try_from(1).unwrap(),
            participant: "participant".parse().unwrap(),
            wire_handles: Default::default(),
            local_capabilities: json!({}),
            server_capabilities: json!({}),
            reserved_handles: [7].into(),
            generations: Default::default(),
            gates: Default::default(),
            selectors: Default::default(),
            reconciliation: None,
            prepared: Some(codecs::Prepared {
                key: key.clone(),
                expected: 1,
                selected: selected.clone(),
                base: vec![],
                wire_handles: [(stream.id.clone(), 7)].into(),
                generations: [(stream.id.clone(), 1)].into(),
                gates: Default::default(),
                observed_revision: StreamRevision::try_from(2).unwrap(),
                video_epochs: Default::default(),
            }),
        };
        let cache =
            std::sync::Mutex::new([(key.scope.connection_id.clone().unwrap(), entry)].into());
        let media = media::Inbox::default();
        let original =
            crate::canonical::Envelope::from_value(notice_value(11, 2, 100, 3000)).unwrap();
        assert!(frame(&cache, &media, &original).unwrap());
        assert_eq!(media.stats().unwrap().queued_packets, 0);
        assert_eq!(
            cache
                .lock()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .prepared
                .as_ref()
                .unwrap()
                .video_epochs
                .len(),
            1
        );
        // Wrong logical root, stale signed revision and wrong allocation cannot
        // displace the pending exact notice. Another client has separate state.
        let other = std::sync::Mutex::new(NativeCache::new());
        assert!(frame(&other, &media, &original).unwrap());
        assert!(other.lock().unwrap().is_empty());
        for raw in [
            {
                let mut v = notice_value(12, 3, 101, 6000);
                v["cid"] = json!("different-room");
                v
            },
            notice_value(12, 1, 101, 6000),
            {
                let mut v = notice_value(12, 3, 101, 6000);
                v["payload"]["original_offer"] = json!("wrong-offer");
                v
            },
        ] {
            assert!(frame(
                &cache,
                &media,
                &crate::canonical::Envelope::from_value(raw).unwrap()
            )
            .unwrap());
        }
        assert_eq!(
            cache
                .lock()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .prepared
                .as_ref()
                .unwrap()
                .video_epochs
                .values()
                .next()
                .unwrap()
                .tag,
            11
        );
        // Use the production paired-capture adoption path, not direct inbox setup.
        let observed = connection_updates::Observed {
            ice: None,
            generation: 1,
            revision: StreamRevision::try_from(2).unwrap(),
            action: None,
            gates: vec![],
            quality: None,
            resources_changed: true,
            codecs: Some(vec![connect_protocol::native_streams::CodecBinding {
                stream_id: stream.id.as_str().into(),
                medium: "video".into(),
                direction: "sendonly".into(),
                stream_local_id: 7,
                stream_generation: 1,
                connection_generation: 1,
                codec: stream.codec.clone(),
            }]),
        };
        codecs::record(&cache, &media, &key, &observed).unwrap();
        assert!(cache
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .prepared
            .is_none());
        let mut rtp = vec![0x80, 96, 0, 100];
        rtp.extend_from_slice(&3000u32.to_be_bytes());
        rtp.extend_from_slice(&83u32.to_be_bytes());
        rtp.extend_from_slice(&[0x10, 0x31]);
        let bytes =
            rvoip_uctp::substrate::tag_video_epoch_datagram(&streams::frame(7, 100, &rtp), 11)
                .unwrap();
        media.push(bytes.to_vec()).unwrap();
        let stream_key = streams::StreamKey {
            conversation_id: "room".parse().unwrap(),
            session_id: "session".parse().unwrap(),
            connection_id: "connection".parse().unwrap(),
            stream_id: stream.id,
        };
        assert!(
            matches!(media.receive(&stream_key,RequestOptions::new(Duration::from_secs(1)).unwrap()).await.unwrap(),MediaPacket::Rtp(p) if p.video_epoch().is_some_and(|e| e.revision==2 && e.epoch==format!("{:032x}",11)))
        );
    }
}
