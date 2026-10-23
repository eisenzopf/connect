//! Native record reads compose the original peer fence with exact backend
//! custody. No policy reader is reacquired after a source writer queues.
use super::*;
use crate::StoreInvocationAuthority;
use connect_protocol::Error;
use rvoip_core::{
    ids::SessionId,
    records::{RecordViewError, RecordViewLease, RecordViewRequest},
    AuthenticatedPrincipal,
};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, DispatchError, MAX_DISPATCH_WAIT},
    Envelope, Payload, Scope,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn intent(e: &Envelope) -> connect_protocol::Result<durable::RecordFetchIntent> {
    let Payload::RecordingVconFetch(input) = e.payload() else {
        return Err(Error::forbidden());
    };
    if e.scope(Scope::Connection).is_some()
        || e.scope(Scope::Reply).is_some()
        || e.expected_revision().is_some()
    {
        return Err(Error::forbidden());
    }
    let request = durable::RecordFetchIntent {
        conversation_id: e
            .scope(Scope::Conversation)
            .ok_or_else(Error::forbidden)?
            .into(),
        session_id: e.scope(Scope::Session).ok_or_else(Error::forbidden)?.into(),
        uuid: input.uuid.clone(),
        url: input.url.clone(),
    };
    request.validate()?;
    Ok(request)
}
struct Sources {
    request: RecordViewRequest,
    held: StoreInvocationAuthority,
    allowed: std::collections::BTreeSet<String>,
}
impl RecordViewLease for Sources {
    fn current(
        &self,
        request: &RecordViewRequest,
    ) -> std::result::Result<std::collections::BTreeSet<String>, RecordViewError> {
        // Facts were derived under this same held current authority. Its scopes
        // prevent authorization changes and projection owns exact record custody.
        // This predicate never enters storage or queues another reader.
        if request != &self.request || !self.held.peer_guard().is_some_and(|p| p.current()) {
            return Err(RecordViewError::Authority);
        }
        Ok(self.allowed.clone())
    }
}
pub(crate) async fn authorize(
    store: Arc<Store>,
    e: &Envelope,
    p: &AuthenticatedPrincipal,
    authority: AuthorityLease,
    cancel: CancellationToken,
    references: Option<Arc<rvoip_uctp::v2::recording::RecordReferenceRegistry>>,
) -> std::result::Result<AuthorityLease, DispatchError> {
    intent(e).map_err(|_| DispatchError::Authority)?;
    let held = authority
        .authority::<StoreInvocationAuthority>()
        .ok_or(DispatchError::Authority)?
        .retained();
    let query_store = store.clone();
    let (origin, policy) = tokio::task::spawn_blocking(move || {
        let proof = held.peer_guard().ok_or_else(Error::forbidden)?;
        let credential = held.binding().credential_id();
        Ok::<_, Error>((
            query_store.peer_record_origin(proof, credential)?,
            query_store.peer_record_policy(proof, credential)?,
        ))
    })
    .await
    .map_err(|_| DispatchError::Authority)?
    .map_err(|_| DispatchError::Authority)?;
    let scope = VconScope::new(
        TenantId::from_string(origin.scope.tenant_id),
        ConversationId::from_string(origin.scope.conversation_id),
        SessionId::from_string(origin.scope.session_id),
    );
    let viewer = viewer(origin.viewer);
    rvoip_uctp::v2::recording::authorize_record_fetch(
        e,
        p,
        scope,
        viewer,
        authority,
        &Records::for_fetch(store, references.clone()),
        move |request, held| {
            let policy = policy.ok_or(DispatchError::Authority)?;
            let binding = view_binding(request, &policy).ok_or(DispatchError::Authority)?;
            if binding != policy.binding {
                return Err(DispatchError::Authority);
            }
            let authority = held
                .authority::<StoreInvocationAuthority>()
                .ok_or(DispatchError::Authority)?
                .retained();
            Ok(Box::new(Sources {
                request: request.clone(),
                held: authority,
                allowed: policy.sources,
            }))
        },
        cancel,
        tokio::time::Instant::now() + MAX_DISPATCH_WAIT,
        references.filter(|r| r.is_available()),
        128 * 1024,
    )
    .await
}
