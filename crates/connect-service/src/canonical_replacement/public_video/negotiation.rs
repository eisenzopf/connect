//! Cold same-Read role/profile derivation. Never a late JSON opt-in.
use super::*;
use crate::canonical_webrtc::ReservedReplacementVideo;
use rvoip_core::staged_streams::device_replacement::ReplacementVideoRequest;
impl Preparation {
    pub(in crate::canonical_replacement) fn negotiation_from_allocation(
        &self,
        read: &connect_store::NativeReplacementAllocationRead,
        record: &str,
    ) -> Result<Option<Arc<ReservedReplacementVideo>>> {
        let offer = read.target_intent()?.offer;
        if !self.required() {
            if offer.get("webrtc_video_receive_reservation").is_some() {
                return Err(Error::forbidden());
            }
            return Ok(None);
        }
        if !read.current()
            || read.source().context().target_transport
                != connect_protocol::provisioning::EndpointTransport::Webrtc
        {
            return Err(Error::forbidden());
        }
        Ok(Some(ReservedReplacementVideo::from_recorded(
            self.reservation()?.clone(),
            record,
            &offer,
        )?))
    }
    pub(in crate::canonical_replacement) fn private_preview_request(
        &self,
        request: ReplacementVideoRequest,
    ) -> Result<ReplacementVideoRequest> {
        if !self.required() {
            return Ok(request);
        }
        private_preview_request(request, self.reservation()?.intent())
    }
}
fn private_preview_request(
    request: ReplacementVideoRequest,
    intent: &connect_protocol::webrtc_video_receive_reservation::Intent,
) -> Result<ReplacementVideoRequest> {
    intent.validate()?;
    if request.receivers.len() != 1
        || request.receivers[0].replacement.as_str() != intent.public_stream_id
        || request.publications.len() != 1
        || request.publications[0].replacement.as_str() != intent.preview_stream_id
    {
        return Err(Error::forbidden());
    }
    let mut receiver = request.receivers[0].clone();
    receiver.replacement = rvoip_core::ids::StreamId::from_string(&intent.preview_stream_id);
    // Local private setup ONLY. The final original plan is untouched.
    Ok(ReplacementVideoRequest {
        publications: vec![],
        receivers: vec![receiver],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_core::{ids::StreamId, staged_streams::device_replacement::ReplacementVideoStream};
    #[test]
    fn private_preview_mapping_is_local_a_only_and_preserves_final_b_plan_and_default_request() {
        let intent = connect_protocol::webrtc_video_receive_reservation::Intent {
            profile: "connect/webrtc-public-video-reservation/1".into(),
            phase: "intent".into(),
            reservation_id: "original-reservation".into(),
            nonce: "original-nonce".into(),
            preview_stream_id: "preview-a".into(),
            preview_role: "receive".into(),
            public_stream_id: "public-b".into(),
        };
        let final_request = ReplacementVideoRequest {
            publications: vec![ReplacementVideoStream {
                previous: StreamId::from_string("old-camera"),
                replacement: StreamId::from_string("preview-a"),
            }],
            receivers: vec![ReplacementVideoStream {
                previous: StreamId::from_string("old-public"),
                replacement: StreamId::from_string("public-b"),
            }],
        };
        let unchanged = final_request.clone();
        let preview = private_preview_request(final_request.clone(), &intent).unwrap();
        assert_eq!(final_request, unchanged);
        assert!(preview.publications.is_empty());
        assert_eq!(preview.receivers.len(), 1);
        assert_eq!(
            preview.receivers[0].previous,
            final_request.receivers[0].previous
        );
        assert_eq!(preview.receivers[0].replacement.as_str(), "preview-a");
        assert_eq!(final_request.receivers[0].replacement.as_str(), "public-b");
        assert_eq!(
            Preparation::new(false)
                .private_preview_request(final_request.clone())
                .unwrap(),
            final_request
        );
        assert!(
            Preparation::new(true)
                .private_preview_request(final_request.clone())
                .is_err(),
            "wire intent cannot replace missing actual Store reservation"
        );
        for mode in 0..3 {
            let mut bad = final_request.clone();
            match mode {
                0 => bad.receivers[0].replacement = StreamId::from_string("foreign-b"),
                1 => bad.publications.clear(),
                _ => bad.receivers.push(bad.receivers[0].clone()),
            };
            assert!(private_preview_request(bad, &intent).is_err());
        }
    }
}
