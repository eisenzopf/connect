//! Pure canonical projection for Store-selected task/provider record sources.
//! Delivery ownership remains with the existing actual host authority read.
use connect_protocol::{Error, Result};
use connect_store::records::{RecordContextData, RecordContextInput, RecordContextProjector};
use rvoip_core::{
    ids::{ConversationId, IdentityId, ParticipantId, SessionId, TenantId},
    records::{
        project_context_snapshot, RecordManifest, RecordViewRequest, RecordViewer, VconRecord,
    },
    store::VconScope,
};

pub(crate) struct CanonicalRecordContext;
impl RecordContextProjector for CanonicalRecordContext {
    fn project(&self, input: RecordContextInput) -> Result<RecordContextData> {
        let invalid = |_| {
            Error::new(
                "record-unavailable",
                "Canonical record context is unavailable",
            )
        };
        let record = VconRecord::new(
            input.bytes.body.into(),
            RecordManifest::decode(&input.bytes.manifest).map_err(invalid)?,
        )
        .map_err(invalid)?;
        if record.uuid().to_string() != input.bytes.uuid {
            return Err(Error::forbidden());
        }
        let scope = VconScope::new(
            TenantId::from_string(input.scope.tenant_id),
            ConversationId::from_string(input.scope.conversation_id),
            SessionId::from_string(input.scope.session_id),
        );
        let viewer = match input.viewer {
            connect_store::records::RecordViewer::Participant {
                participant_id,
                membership_epoch,
            } => RecordViewer::Participant {
                participant: ParticipantId::from_string(participant_id),
                membership_epoch,
            },
            connect_store::records::RecordViewer::TenantRole {
                identity_id,
                tenant_id,
                role,
                revision,
                private_sources,
            } => RecordViewer::TenantRole {
                identity: IdentityId::from_string(identity_id),
                tenant: TenantId::from_string(tenant_id),
                role,
                revision,
                private_sources,
            },
        };
        let request = RecordViewRequest::new(
            scope,
            viewer,
            record.uuid(),
            input.handle.revision,
            input.handle.body_hash,
            input.handle.manifest_hash,
            input.correlation_id,
        )
        .map_err(|_| Error::forbidden())?;
        let snapshot = project_context_snapshot(&record, &request, &input.allowed_sources)
            .map_err(|_| {
                Error::new(
                    "record-unavailable",
                    "Canonical record context is unavailable",
                )
            })?;
        Ok(RecordContextData {
            body: snapshot.body().to_vec(),
            content_hash: snapshot.content_hash().to_owned(),
            valid_until: snapshot.valid_until().map(|v| v.timestamp()),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use rvoip_core::records::{
        RecordAudience, RecordCategory, RecordRetention, RecordSource, RecordTarget,
    };
    use serde_json::json;
    use sha2::{Digest, Sha512};
    fn digest(bytes: impl AsRef<[u8]>) -> String {
        format!("sha512-{}", URL_SAFE_NO_PAD.encode(Sha512::digest(bytes)))
    }

    fn fixture() -> (RecordContextInput, i64) {
        let mut uuid = *uuid::Uuid::new_v4().as_bytes();
        uuid[6] = (uuid[6] & 15) | 128;
        let uuid = uuid::Uuid::from_bytes(uuid);
        let until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let body=serde_json::to_vec(&json!({"vcon":"0.4.0","uuid":uuid,"created_at":chrono::Utc::now(),"subject":"record context",
            "analysis":[
                {"type":"summary","vendor":"fixture","body":"PUBLIC-CONTEXT-SOURCE","encoding":"none","mediatype":"text/plain"},
                {"type":"summary","vendor":"fixture","body":"PRIVATE-CONTEXT-CANARY","encoding":"none","mediatype":"text/plain"}
            ]})).unwrap();
        let source = |id: &str, target, audience, content| {
            RecordSource::new(
                id.into(),
                1,
                target,
                RecordCategory::Metadata,
                audience,
                "context-fixture".into(),
                RecordRetention::UntilDeleted,
                content,
            )
        };
        let manifest = RecordManifest::new(
            "context-fixture".into(),
            vec![
                source(
                    "document",
                    RecordTarget::Document,
                    RecordAudience::Policy,
                    RecordRetention::UntilDeleted,
                ),
                source(
                    "summary.public",
                    RecordTarget::Analysis(0),
                    RecordAudience::Policy,
                    RecordRetention::Until(until),
                ),
                source(
                    "summary.private",
                    RecordTarget::Analysis(1),
                    RecordAudience::Participants(vec![ParticipantId::from_string("part_private")]),
                    RecordRetention::Until(chrono::Utc::now() + chrono::Duration::seconds(3)),
                ),
            ],
        )
        .unwrap();
        let manifest_hash = digest(manifest.encode().unwrap());
        let handle = connect_store::records::RecordHandle {
            location: format!("connect:vcon/{uuid}"),
            body_hash: digest(&body),
            manifest_hash,
            revision: 7,
        };
        (
            RecordContextInput {
                scope: connect_store::records::RecordScope {
                    tenant_id: "tenant_context".into(),
                    conversation_id: "conversation_context".into(),
                    session_id: "session_context".into(),
                },
                handle,
                bytes: connect_store::records::RecordBytes {
                    uuid: uuid.to_string(),
                    body,
                    manifest: manifest.encode().unwrap(),
                },
                viewer: connect_store::records::RecordViewer::Participant {
                    participant_id: "part_public".into(),
                    membership_epoch: 2,
                },
                allowed_sources: ["document", "summary.public", "summary.private"]
                    .map(str::to_owned)
                    .into(),
                correlation_id: "context-fixture".into(),
            },
            until.timestamp(),
        )
    }
    #[test]
    fn canonical_context_bridge_preserves_exact_projection_attribution_and_visible_deadline() {
        let (input, until) = fixture();
        let source_uuid = input.bytes.uuid.clone();
        let data = CanonicalRecordContext.project(input).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&data.body).unwrap();
        assert_eq!(value["uuid"], source_uuid);
        assert_eq!(data.content_hash, digest(&data.body));
        assert_eq!(data.valid_until, Some(until));
        let text = String::from_utf8(data.body.clone()).unwrap();
        assert!(text.contains("PUBLIC-CONTEXT-SOURCE"));
        assert!(!text.contains("PRIVATE-CONTEXT-CANARY"));
        assert!(text.contains("summary.public"));
        assert!(!text.contains("summary.private"));
        assert!(text.contains("context-fixture"));
        assert!(!format!("{data:?}").contains("PUBLIC-CONTEXT-SOURCE"));
    }
    #[test]
    fn canonical_context_bridge_refuses_changed_bytes_manifest_and_cross_tenant_role() {
        let (mut input, _) = fixture();
        input.bytes.body.push(b' ');
        assert!(CanonicalRecordContext.project(input).is_err());
        let (mut input, _) = fixture();
        input.handle.manifest_hash = input.handle.body_hash.clone();
        assert!(CanonicalRecordContext.project(input).is_err());
        let (mut input, _) = fixture();
        input.viewer = connect_store::records::RecordViewer::TenantRole {
            identity_id: "identity_context".into(),
            tenant_id: "different_tenant".into(),
            role: "auditor".into(),
            revision: 1,
            private_sources: true,
        };
        assert!(CanonicalRecordContext.project(input).is_err());
    }
}
