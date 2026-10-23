//! One immutable host policy shared by signed peers and the HTTP facade.
use connect_protocol::{user_delegation::UserAssuranceRule, Error, Result};
use rvoip_core::{AuthenticatedPrincipal, IdentityAssurance};
use rvoip_uctp::v2::{auth_peer::AssuranceRequirement, Envelope, Scope};

#[derive(Default)]
pub(crate) struct Policy {
    rules: Vec<UserAssuranceRule>,
}
impl Policy {
    pub(crate) fn new(rules: Vec<UserAssuranceRule>) -> Result<Self> {
        if rules.len() > 64 {
            return Err(Error::new(
                "invalid-request",
                "Assurance policy capacity exceeded",
            ));
        }
        let mut keys = std::collections::BTreeSet::new();
        for rule in &rules {
            rule.validate()?;
            if !keys.insert((&rule.conversation_id, &rule.command)) {
                return Err(Error::new("invalid-request", "Ambiguous assurance policy"));
            }
        }
        Ok(Self { rules })
    }
    pub(crate) fn requirement(&self, e: &Envelope) -> Option<AssuranceRequirement> {
        let rule = self.rules.iter().find(|rule| {
            e.scope(Scope::Conversation) == Some(rule.conversation_id.as_str())
                && e.kind() == rule.command
        })?;
        Some(AssuranceRequirement::UserAuthorized {
            user_id: rvoip_core::IdentityId::from_string(&rule.user_id),
            scopes: vec![rule.required_operation().ok()?.to_owned()],
        })
    }
    pub(crate) fn permits(&self, e: &Envelope, p: &AuthenticatedPrincipal) -> bool {
        let Some(required) = self.requirement(e) else {
            return true;
        };
        match (required, &p.assurance) {
            (
                AssuranceRequirement::UserAuthorized { user_id, scopes },
                IdentityAssurance::UserAuthorized {
                    identity,
                    user_id: actual,
                    scopes: granted,
                },
            ) => {
                identity.as_str() == p.subject
                    && user_id == *actual
                    && scopes.iter().all(|s| p.has_scope(s) && granted.contains(s))
            }
            _ => false,
        }
    }
}
impl crate::Service {
    pub async fn canonical_peer_authority(
        &self,
        capacity: usize,
        record_references: Option<
            std::sync::Arc<rvoip_uctp::v2::recording::RecordReferenceRegistry>,
        >,
    ) -> std::result::Result<
        std::sync::Arc<crate::StorePeerAuthority>,
        rvoip_uctp::v2::auth_dispatch::DispatchError,
    > {
        crate::StorePeerAuthority::with_assurance(
            self.store.clone(),
            capacity,
            record_references,
            self.assurance_policy.get().cloned().unwrap_or_default(),
            self.canonical_runtime
                .get()
                .map(std::sync::Arc::downgrade)
                .unwrap_or_default(),
        )
        .await
    }
    /// Install before canonical runtime/intake. Policy is immutable for this
    /// incarnation, so the synchronous peer floor callback takes no Store locks.
    pub fn install_canonical_assurance(&self, rules: Vec<UserAssuranceRule>) -> Result<()> {
        if self.canonical_runtime.get().is_some() {
            return Err(Error::new(
                "conflict",
                "Install assurance policy before canonical runtime",
            ));
        }
        let policy = std::sync::Arc::new(Policy::new(rules)?);
        self.assurance_policy
            .set(policy)
            .map_err(|_| Error::new("conflict", "Assurance policy already installed"))
    }
}
