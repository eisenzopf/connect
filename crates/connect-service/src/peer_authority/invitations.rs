//! Exact idle physical binding metadata, then one fresh Store read union for
//! actual invitation IO. No historical grant is retained in the route origin.
use super::*;
use rvoip_uctp::v2::reachability::{RouteAuthority, RouteOrigin};
#[async_trait]
impl RouteAuthority for StorePeerAuthority {
    fn origin(
        &self,
        c: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        held: &AuthorityLease,
    ) -> Result<RouteOrigin, DispatchError> {
        let authority = held
            .authority::<StoreInvocationAuthority>()
            .ok_or(DispatchError::Authority)?;
        let grant = authority
            .guard::<Arc<Grant>>()
            .ok_or(DispatchError::Authority)?;
        if grant.policy != self.id
            || !grant.proof.current()
            || self.stop.is_cancelled()
            || !authority.binding().matches(&c.principal)
            || c.token_id.as_deref() != Some(authority.binding().credential_id())
            || c.issued_at != grant.credential.issued_at
            || grant.request.device != device(d)
            || i.as_str() != c.principal.subject
        {
            return Err(DispatchError::Authority);
        }
        let mut request = grant.request.clone();
        request.credentials = vec![authority.binding().credential_id().into()];
        request.command = PeerCommand::Binding;
        // The transport separately retains the exact existing AuthorityLifetime,
        // including its key/credential retirement watch. This is metadata only.
        Ok(RouteOrigin::new(self.stop.child_token(), request))
    }
    async fn delivery(
        &self,
        c: &ValidatedBearer,
        d: &Device,
        i: &Identifier,
        origin: &RouteOrigin,
        e: &Envelope,
    ) -> Result<AuthorityLease, DispatchError> {
        let mut request = origin
            .metadata::<PeerAuthorityRequest>()
            .ok_or(DispatchError::Authority)?
            .clone();
        if e.scope(Scope::Connection).is_some()
            || e.expected_revision().is_some()
            || request.credentials.as_slice()
                != [c.token_id.clone().ok_or(DispatchError::Authority)?]
            || request.device != device(d)
            || c.principal.subject != i.as_str()
        {
            return Err(DispatchError::Authority);
        }
        if matches!(
            e.payload(),
            Payload::SessionParticipantJoined(_) | Payload::SessionParticipantLeft(_)
        ) {
            if e.scope(Scope::Reply).is_some() || e.scope(Scope::Operation).is_some() {
                return Err(DispatchError::Authority);
            }
            request.command = PeerCommand::SessionParticipantFactRead {
                conversation_id: e
                    .scope(Scope::Conversation)
                    .ok_or(DispatchError::Authority)?
                    .into(),
                session_id: e
                    .scope(Scope::Session)
                    .ok_or(DispatchError::Authority)?
                    .into(),
                fact_id: e
                    .id()
                    .split_once('.')
                    .filter(|(id, nonce)| {
                        id.starts_with("spf_")
                            && id.len() == 36
                            && nonce.len() == 32
                            && nonce.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                    .ok_or(DispatchError::Authority)?
                    .0
                    .into(),
            };
            let store = self.store.clone();
            let credential = c.clone();
            let expected = e.original()["payload"].clone();
            let expected_kind = e.kind().to_owned();
            let stop = self.stop.clone();
            let wait = AuthorityWait::new(Duration::from_secs(2));
            let _cancel = wait.cancel_on_drop();
            return tokio::task::spawn_blocking(move || {
                let proof = Arc::new(
                    store
                        .admit_peer_authority(&request, &wait)
                        .map_err(store_error)?,
                );
                let binding = StoreCredentialBinding::from_validated(&store, &credential)
                    .map_err(store_error)?;
                let fact = store
                    .session_participant_fact_with_peer(binding.credential_id(), &proof)
                    .map_err(store_error)?;
                let hint = crate::canonical_session_lifecycle::participant_hint(&fact, None)
                    .map_err(store_error)?;
                if hint.kind() != expected_kind
                    || hint.original()["payload"] != expected
                    || stop.is_cancelled()
                {
                    return Err(DispatchError::Authority);
                }
                let expires = chrono::DateTime::from_timestamp(proof.expires_at(), 0)
                    .ok_or(DispatchError::Authority)?;
                let remaining = (expires - chrono::Utc::now())
                    .to_std()
                    .map_err(|_| DispatchError::Authority)?;
                let current = proof.clone();
                Ok(AuthorityLease::checking(
                    (proof, binding),
                    tokio::time::Instant::now() + remaining,
                    move || current.current() && !stop.is_cancelled(),
                ))
            })
            .await
            .map_err(|_| DispatchError::Authority)?;
        }
        if matches!(
            e.payload(),
            Payload::SessionStarted(_) | Payload::SessionEnded(_) | Payload::RecordingVconReady(_)
        ) {
            if e.scope(Scope::Reply).is_some() || e.scope(Scope::Operation).is_some() {
                return Err(DispatchError::Authority);
            }
            let cid = e
                .scope(Scope::Conversation)
                .ok_or(DispatchError::Authority)?
                .to_owned();
            let sid = e
                .scope(Scope::Session)
                .ok_or(DispatchError::Authority)?
                .to_owned();
            let kind = e.kind().to_owned();
            request.command = match kind.as_str() {
                "session.ended" => PeerCommand::SessionEndedRead {
                    conversation_id: cid,
                    session_id: sid,
                },
                "session.started" => PeerCommand::SessionStartedRead {
                    conversation_id: cid,
                    session_id: sid,
                },
                "recording.vcon-ready" => {
                    PeerCommand::RecordReadyRead(connect_store::records::RecordFetchIntent {
                        conversation_id: cid,
                        session_id: sid,
                        uuid: Some(
                            e.original()["payload"]["vcon_handle"]["uuid"]
                                .as_str()
                                .ok_or(DispatchError::Authority)?
                                .into(),
                        ),
                        url: None,
                    })
                }
                _ => return Err(DispatchError::Authority),
            };
            let store = self.store.clone();
            let credential = c.clone();
            let payload = e.original()["payload"].clone();
            let stop = self.stop.clone();
            let wait = AuthorityWait::new(Duration::from_secs(2));
            let _cancel = wait.cancel_on_drop();
            return tokio::task::spawn_blocking(move || {
                let proof = Arc::new(
                    store
                        .admit_peer_authority(&request, &wait)
                        .map_err(store_error)?,
                );
                let binding = StoreCredentialBinding::from_validated(&store, &credential)
                    .map_err(store_error)?;
                let actual = match kind.as_str() {
                    "session.ended" => {
                        let fact = store
                            .session_ended_with_peer(binding.credential_id(), &proof)
                            .map_err(store_error)?;
                        crate::canonical_session_lifecycle::ended_payload(&fact)
                            .map_err(store_error)?
                    }
                    "session.started" => {
                        let fact = store
                            .session_started_with_peer(binding.credential_id(), &proof)
                            .map_err(store_error)?;
                        crate::canonical_session_lifecycle::started_payload(&fact)
                            .map_err(store_error)?
                    }
                    "recording.vcon-ready" => {
                        let fact = store
                            .record_ready_with_peer(binding.credential_id(), &proof)
                            .map_err(store_error)?;
                        crate::canonical_session_lifecycle::record_ready_payload(&fact)
                            .map_err(store_error)?
                    }
                    _ => return Err(DispatchError::Authority),
                };
                if actual != payload || stop.is_cancelled() {
                    return Err(DispatchError::Authority);
                }
                let expires = chrono::DateTime::from_timestamp(proof.expires_at(), 0)
                    .ok_or(DispatchError::Authority)?;
                let remaining = (expires - chrono::Utc::now())
                    .to_std()
                    .map_err(|_| DispatchError::Authority)?;
                let current = proof.clone();
                Ok(AuthorityLease::checking(
                    (proof, binding),
                    tokio::time::Instant::now() + remaining,
                    move || current.current() && !stop.is_cancelled(),
                ))
            })
            .await
            .map_err(|_| DispatchError::Authority)?;
        }
        let cancellation = matches!(e.payload(), Payload::SessionCancel(_));
        if cancellation && e.scope(Scope::Reply).is_none() {
            return Err(DispatchError::Authority);
        }
        let relay = matches!(
            e.payload(),
            Payload::SessionAccept(_) | Payload::SessionReject(_)
        );
        let source = if relay {
            if e.scope(Scope::Reply).is_none() {
                return Err(DispatchError::Authority);
            }
            let by = match e.payload() {
                Payload::SessionAccept(v) => v.by.as_ref(),
                Payload::SessionReject(v) => v.by.as_ref(),
                _ => None,
            }
            .ok_or(DispatchError::Authority)?;
            self.store
                .session_invitation_caller_source(
                    i.as_str(),
                    e.scope(Scope::Operation).ok_or(DispatchError::Authority)?,
                    by.as_str(),
                )
                .map_err(store_error)?
        } else {
            if !cancellation
                && (!matches!(e.payload(), Payload::SessionInvite(_))
                    || e.scope(Scope::Reply).is_some())
            {
                return Err(DispatchError::Unsupported);
            }
            self.store
                .session_invitation_target(
                    e.scope(Scope::Operation).ok_or(DispatchError::Authority)?,
                    i.as_str(),
                )
                .map_err(store_error)?
        };
        request.command = if cancellation {
            PeerCommand::SessionInvitationCancellationRead(source.clone())
        } else if relay {
            PeerCommand::SessionInvitationRelayRead(source.clone())
        } else {
            PeerCommand::SessionInvitationRead(source.clone())
        };
        let store = self.store.clone();
        let credential = c.clone();
        let envelope = e.clone();
        let stop = self.stop.clone();
        let wait = AuthorityWait::new(Duration::from_secs(2));
        let _cancel = wait.cancel_on_drop();
        tokio::task::spawn_blocking(move || {
            let proof = Arc::new(
                store
                    .admit_peer_authority(&request, &wait)
                    .map_err(store_error)?,
            );
            let binding =
                StoreCredentialBinding::from_validated(&store, &credential).map_err(store_error)?;
            let valid = if cancellation {
                let original = store
                    .session_invitation_cancellation_with_peer(
                        &source,
                        binding.credential_id(),
                        &proof,
                    )
                    .map_err(store_error)?;
                let Payload::SessionCancel(value) = envelope.payload() else {
                    return Err(DispatchError::Authority);
                };
                envelope.scope(Scope::Conversation) == Some(original.conversation_id.as_str())
                    && envelope.scope(Scope::Session) == Some(original.session_id.as_str())
                    && envelope.scope(Scope::Operation) == Some(source.invitation_id())
                    && value.by.as_ref().map(|id| id.as_str())
                        == Some(original.by_participant_id.as_str())
                    && value.reason_code == original.reason_code
                    && value.reason == original.reason
            } else if relay {
                let (operation, outcome) = store
                    .session_invitation_relay_with_peer(&source, binding.credential_id(), &proof)
                    .map_err(store_error)?;
                envelope.scope(Scope::Conversation)
                    == Some(outcome.session.conversation_id.as_str())
                    && envelope.scope(Scope::Session) == Some(outcome.session.id.as_str())
                    && envelope.scope(Scope::Operation) == Some(operation.as_str())
                    && match (envelope.payload(), &outcome.decision) {
                        (
                            Payload::SessionAccept(value),
                            connect_store::SessionInvitationDecision::Accept {
                                capabilities, ..
                            },
                        ) => {
                            value.by.as_ref().map(|id| id.as_str())
                                == Some(outcome.participant_id.as_str())
                                && value.capabilities_answer == *capabilities
                        }
                        (
                            Payload::SessionReject(value),
                            connect_store::SessionInvitationDecision::Reject {
                                reason_code,
                                reason,
                                ..
                            },
                        ) => {
                            value.by.as_ref().map(|id| id.as_str())
                                == Some(outcome.participant_id.as_str())
                                && value.reason_code == *reason_code
                                && value.reason == *reason
                        }
                        _ => false,
                    }
            } else {
                let metadata = store
                    .session_invitation_metadata_with_peer(&source, binding.credential_id(), &proof)
                    .map_err(store_error)?;
                let Payload::SessionInvite(invite) = envelope.payload() else {
                    return Err(DispatchError::Authority);
                };
                envelope.scope(Scope::Conversation) == Some(metadata.conversation_id.as_str())
                    && envelope.scope(Scope::Session) == Some(metadata.session_id.as_str())
                    && invite.r#from.as_ref().map(|id| id.as_str())
                        == Some(metadata.from_participant_id.as_str())
                    && invite.to.len() == 1
                    && invite.to[0].as_str() == metadata.target_participant_id
                    && invite.medium == metadata.medium
                    && invite.intent == metadata.intent
                    && invite.capabilities_offer == metadata.capabilities_offer
            };
            if !valid || stop.is_cancelled() {
                return Err(DispatchError::Authority);
            }
            let expires = chrono::DateTime::from_timestamp(proof.expires_at(), 0)
                .ok_or(DispatchError::Authority)?;
            let remaining = (expires - chrono::Utc::now())
                .to_std()
                .map_err(|_| DispatchError::Authority)?;
            let current = proof.clone();
            Ok(AuthorityLease::checking(
                (proof, binding),
                tokio::time::Instant::now() + remaining,
                move || current.current() && !stop.is_cancelled(),
            ))
        })
        .await
        .map_err(|_| DispatchError::Authority)?
    }
}
