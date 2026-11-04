//! IPC holds an opaque reference to the original Rust hint, never a renewed
//! cross-process lifetime. One-shot consumption rechecks the original Client.
use super::*;
use std::{collections::BTreeMap, sync::Mutex};

const CAPACITY: usize = 128;
struct Entry {
    token: String,
    request: VideoKeyframeRequest,
}
#[derive(Default)]
pub(super) struct Leases(Mutex<BTreeMap<streams::StreamKey, Entry>>);

fn key(binding: &Binding, stream: &StreamId) -> streams::StreamKey {
    streams::StreamKey {
        conversation_id: binding.conversation_id.clone(),
        session_id: binding.session_id.clone(),
        connection_id: binding.connection_id.clone(),
        stream_id: stream.clone(),
    }
}
fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && token.bytes().any(|b| b != b'0')
}
impl Leases {
    pub fn insert(&self, request: VideoKeyframeRequest) -> Result<String> {
        if request.expires_in().is_zero() {
            return Err(Error::local(ErrorKind::Deadline));
        }
        let mut entries = self
            .0
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        entries.retain(|_, e| !e.request.expires_in().is_zero());
        if entries.len() >= CAPACITY && !entries.contains_key(&request.key) {
            return Err(Error::local(ErrorKind::Conflict));
        }
        let token = uuid::Uuid::new_v4().simple().to_string();
        entries.insert(
            request.key.clone(),
            Entry {
                token: token.clone(),
                request,
            },
        );
        Ok(token)
    }
    pub fn consume(
        &self,
        binding: &Binding,
        stream: &StreamId,
        token: &str,
        current: impl FnOnce(&VideoKeyframeRequest) -> Result<bool>,
    ) -> Result<u16> {
        if !valid_token(token) {
            return Err(Error::local(ErrorKind::Validation));
        }
        let mut entries = self
            .0
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        entries.retain(|_, e| !e.request.expires_in().is_zero());
        let key = key(binding, stream);
        if entries.get(&key).is_none_or(|e| e.token != token) {
            return Ok(0);
        }
        let entry = entries.remove(&key).expect("matched lease");
        // The reference is consumed even if observation or response is lost.
        // Uncertainty is never permission to replay a keyframe hint.
        if !current(&entry.request)? {
            return Ok(0);
        }
        Ok(entry.request.expires_in().as_millis().min(200) as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk::video_feedback::epochs::{
        tests::{notice_value, selected},
        Notice,
    };
    use rvoip_uctp::substrate::{pack_video_feedback_datagram, VideoFeedbackDatagram};
    fn binding() -> Binding {
        Binding {
            conversation_id: "room".parse().unwrap(),
            session_id: "session".parse().unwrap(),
            connection_id: "connection".parse().unwrap(),
        }
    }
    fn inbox() -> crate::sdk::media::Inbox {
        let inbox = crate::sdk::media::Inbox::default();
        let mut selected = selected();
        selected.streams[0].direction = Direction::Sendonly;
        let b = binding();
        inbox
            .install(
                &b.conversation_id,
                &b.session_id,
                &b.connection_id,
                &selected,
            )
            .unwrap();
        let mut raw = notice_value(11, 1, 100, 3000);
        raw["payload"]["direction"] = json!("publisher");
        inbox
            .install_epoch(
                Notice::read(&crate::canonical::Envelope::from_value(raw).unwrap()).unwrap(),
            )
            .unwrap();
        inbox
    }
    async fn hint(inbox: &crate::sdk::media::Inbox, sequence: u32) -> VideoKeyframeRequest {
        inbox
            .push(
                pack_video_feedback_datagram(&VideoFeedbackDatagram {
                    stream_local_id: 7,
                    seq: sequence,
                    sender_ssrc: 1,
                    media_ssrc: 83,
                    activation_epoch: 11,
                })
                .unwrap()
                .to_vec(),
            )
            .unwrap();
        inbox
            .receive_feedback(
                &key(&binding(), &"camera".parse().unwrap()),
                RequestOptions::new(Duration::from_secs(1)).unwrap(),
            )
            .await
            .unwrap()
    }
    #[tokio::test(start_paused = true)]
    async fn helper_feedback_lease_retains_original_age_and_consumes_once_on_current_binding() {
        let inbox = inbox();
        let leases = Leases::default();
        let b = binding();
        let stream = "camera".parse().unwrap();
        let k = key(&b, &stream);
        let request = hint(&inbox, 0).await;
        tokio::time::advance(Duration::from_millis(150)).await;
        let token = leases.insert(request).unwrap();
        let mut wrong = binding();
        wrong.connection_id = "another".parse().unwrap();
        assert_eq!(
            leases
                .consume(&wrong, &stream, &token, |_| panic!(
                    "wrong scope reached check"
                ))
                .unwrap(),
            0
        );
        assert_eq!(
            leases
                .consume(&b, &stream, &"f".repeat(32), |_| panic!(
                    "wrong token reached check"
                ))
                .unwrap(),
            0
        );
        tokio::time::advance(Duration::from_millis(20)).await;
        assert_eq!(
            leases
                .consume(&b, &stream, &token, |r| inbox.feedback_is_current(&k, r))
                .unwrap(),
            30
        );
        assert_eq!(
            leases
                .consume(&b, &stream, &token, |_| panic!("consumed lease replay"))
                .unwrap(),
            0
        );
        let token = leases.insert(hint(&inbox, 1).await).unwrap();
        tokio::time::advance(Duration::from_millis(201)).await;
        assert_eq!(
            leases
                .consume(&b, &stream, &token, |_| panic!(
                    "expired lease reached check"
                ))
                .unwrap(),
            0
        );
        let old = leases.insert(hint(&inbox, 2).await).unwrap();
        let new = leases.insert(hint(&inbox, 3).await).unwrap();
        assert_eq!(
            leases
                .consume(&b, &stream, &old, |_| panic!(
                    "superseded lease reached check"
                ))
                .unwrap(),
            0
        );
        inbox
            .apply_directions(&[(k.clone(), Direction::Inactive)])
            .unwrap();
        assert_eq!(
            leases
                .consume(&b, &stream, &new, |r| inbox.feedback_is_current(&k, r))
                .unwrap(),
            0
        );
        assert!(leases.0.lock().unwrap().is_empty());
        let other = Leases::default();
        assert_eq!(
            other
                .consume(&b, &stream, &token, |_| panic!(
                    "foreign helper lease reached check"
                ))
                .unwrap(),
            0
        );
    }
    #[tokio::test(start_paused = true)]
    async fn helper_feedback_lease_capacity_expires_without_extending_original_hints() {
        let inbox = inbox();
        let leases = Leases::default();
        let original = hint(&inbox, 0).await;
        for index in 0..CAPACITY {
            let mut request = original.clone();
            request.key.stream_id = format!("stream-{index}").parse().unwrap();
            leases.insert(request).unwrap();
        }
        assert_eq!(leases.0.lock().unwrap().len(), CAPACITY);
        assert_eq!(
            leases.insert(original.clone()).unwrap_err().kind,
            ErrorKind::Conflict
        );
        tokio::time::advance(Duration::from_millis(201)).await;
        assert_eq!(
            leases.insert(original).unwrap_err().kind,
            ErrorKind::Deadline
        );
        leases.insert(hint(&inbox, 1).await).unwrap();
        assert_eq!(leases.0.lock().unwrap().len(), 1);
    }
}
