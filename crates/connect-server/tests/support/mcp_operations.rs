//! Typed operation fixtures independently define the expected task wire intent.
use connect_protocol::Operation;
use sha2::{Digest, Sha256};

pub fn cases(cid: &str, peer: &str) -> Vec<Operation> {
    let cid = cid.to_owned();
    vec![
        Operation::Resolve {
            address: "connect:agent@local".into(),
        },
        Operation::Capabilities {
            address: "connect:agent@local".into(),
            conversation_id: Some(cid.clone()),
        },
        Operation::Send {
            request_id: "connect_send".into(),
            conversation_id: cid.clone(),
            message: connect_protocol::messages::MessageDraft::text_all("Parity keeps one message"),
        },
        Operation::History {
            conversation_id: cid.clone(),
            after: None,
            limit: 100,
        },
        Operation::MessageReceipts {
            conversation_id: cid.clone(),
            message_id: "unknown".into(),
            after: None,
            limit: 100,
        },
        Operation::ContextView {
            conversation_id: cid.clone(),
            limit: 100,
        },
        Operation::ContextDeliveries {
            conversation_id: cid.clone(),
            before: None,
            limit: 100,
        },
        Operation::ArtifactBegin {
            to: connect_protocol::messages::MessageAudience::all(),
            request_id: "connect_artifact_begin".into(),
            conversation_id: cid.clone(),
            name: "draft.txt".into(),
            media_type: "text/plain".into(),
            size: 1,
            sha256: hex::encode(Sha256::digest(b"a")),
            expires_at: None,
        },
        Operation::ArtifactAppend {
            request_id: "connect_artifact_append".into(),
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
            offset: 0,
            data: "YQ==".into(),
        },
        Operation::ArtifactFinish {
            request_id: "connect_artifact_finish".into(),
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
        },
        Operation::ArtifactGet {
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
            offset: 0,
            limit: 100,
        },
        Operation::ArtifactInspect {
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
        },
        Operation::ArtifactUploadLink {
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
            expires_in: 60,
        },
        Operation::ArtifactDownloadLink {
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
            message_id: "unknown".into(),
            expires_in: 60,
        },
        Operation::ArtifactLinkGet {
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
            link: "invalid".into(),
            offset: 0,
            limit: 100,
        },
        Operation::ArtifactLinkAppend {
            request_id: "connect_artifact_link_append".into(),
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
            link: "invalid".into(),
            offset: 0,
            data: "YQ==".into(),
        },
        Operation::ArtifactDelete {
            request_id: "connect_artifact_delete".into(),
            conversation_id: cid.clone(),
            artifact_id: "unknown".into(),
        },
        Operation::SessionInvite {
            request_id: "connect_session_invite".into(),
            conversation_id: cid.clone(),
            participant_id: peer.into(),
            media: vec!["unsupported".into()],
        },
        Operation::SessionAccept {
            request_id: "connect_session_accept".into(),
            conversation_id: cid.clone(),
            session_id: "unknown".into(),
        },
        Operation::SessionReject {
            request_id: "connect_session_reject".into(),
            conversation_id: cid.clone(),
            session_id: "unknown".into(),
            reason: Some("not now".into()),
        },
        Operation::SessionCancel {
            request_id: "connect_session_cancel".into(),
            conversation_id: cid.clone(),
            session_id: "unknown".into(),
        },
        Operation::SessionEnd {
            request_id: "connect_session_end".into(),
            conversation_id: cid.clone(),
            session_id: "unknown".into(),
            reason: None,
        },
        Operation::SessionGet {
            conversation_id: cid.clone(),
            session_id: "unknown".into(),
        },
        Operation::CallStart {
            request_id: "connect_call_start".into(),
            conversation_id: cid.clone(),
            route_id: "unknown".into(),
            destination: "+15555550101".into(),
        },
        Operation::CallStatus {
            conversation_id: cid.clone(),
            call_id: "unknown".into(),
        },
        Operation::CallEnd {
            request_id: "connect_call_end".into(),
            conversation_id: cid.clone(),
            call_id: "unknown".into(),
        },
        Operation::SipDial {
            request_id: "connect_sip_dial".into(),
            conversation_id: cid.clone(),
            session_id: "unknown".into(),
            destination: "sip:peer@example.invalid".into(),
        },
        Operation::SipStatus {
            conversation_id: cid.clone(),
            sip_leg_id: "unknown".into(),
        },
        Operation::SipEnd {
            request_id: "connect_sip_end".into(),
            conversation_id: cid,
            sip_leg_id: "unknown".into(),
        },
    ]
}
