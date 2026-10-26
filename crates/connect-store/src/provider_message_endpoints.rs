//! Protected canonical message routing binds actual endpoints and account custody.
use crate::{
    identity::actor_from_connection, providers::route, AuthorityMode, AuthorityScope,
    AuthorityWait, Route, Store,
};
use connect_protocol::{membership::MemberView, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMessageEndpoint {
    pub route_id: String,
    pub connector: String,
    pub transport: connect_protocol::provisioning::EndpointTransport,
    pub endpoint_id: String,
    pub source_endpoint_id: Option<String>,
    pub account_credential_id: String,
    pub secret_reference: String,
}
impl std::fmt::Debug for ProviderMessageEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderMessageEndpoint(redacted)")
    }
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Facts {
    route: Route,
    member: MemberView,
    target: Value,
    source: Option<Value>,
    account_owner: String,
}
fn endpoint(c: &Connection, id: &str) -> Result<Value> {
    let raw:String=c.query_row("SELECT json_object('id',e.id,'identity',e.identity_id,'tenant',e.tenant_id,'revision',e.revision,'address',e.address,'transport',e.transport,'capabilities',json(e.capabilities_json),'secret_ref',e.secret_ref,'owner',e.route_owner_id,'device',e.device_id,'kind',d.kind,'platform',d.platform) FROM endpoints e JOIN identities i ON i.id=e.identity_id AND i.revoked=0 JOIN devices d ON d.id=e.device_id AND d.identity_id=i.id AND d.revoked=0 WHERE e.id=?1 AND e.verification_state='verified'",[id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
    serde_json::from_str(&raw).map_err(|_| Error::storage())
}
fn manifest(c: &Connection, connector: &str) -> Result<connect_connector_api::manifest::Manifest> {
    let raw: String = c
        .query_row(
            "SELECT manifest FROM connector_manifests WHERE name=?1",
            [connector],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    let value: connect_connector_api::manifest::Manifest =
        serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    value.validate().map_err(|_| Error::forbidden())?;
    if value.name != connector || !value.messaging {
        return Err(Error::forbidden());
    }
    Ok(value)
}
fn facts(c: &Connection, p: &ProviderMessageEndpoint) -> Result<Facts> {
    for id in [
        &p.route_id,
        &p.endpoint_id,
        &p.account_credential_id,
        &p.secret_reference,
    ]
    .into_iter()
    .chain(p.source_endpoint_id.iter())
    {
        connect_protocol::transitions::id(id)?;
    }
    let mut r = route(c, &p.route_id)?;
    if r.connector != p.connector {
        return Err(Error::forbidden());
    }
    let account = actor_from_connection(c, &p.account_credential_id)?;
    account.require("admin")?;
    let delegate = actor_from_connection(c, &r.credential_id)?;
    delegate.require("send")?;
    let room = crate::conversations::authorized_conversation(c, &delegate, &r.conversation_id)?;
    if room.closed
        || delegate.identity_id != r.identity_id
        || !account.permits_recipient(&r.identity_id)
    {
        return Err(Error::forbidden());
    }
    let member = crate::audience::member(c, &r.identity_id, &r.conversation_id, true)?;
    if member.role == "observer" {
        return Err(Error::forbidden());
    }
    let target = endpoint(c, &p.endpoint_id)?;
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1",
            [&account.identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    if target["tenant"] != tenant {
        return Err(Error::forbidden());
    }
    let declaration = manifest(c, &p.connector)?;
    use connect_connector_api::AddressKind;
    let transport = target["transport"].as_str().ok_or_else(Error::forbidden)?;
    if transport != p.transport.as_str() {
        return Err(Error::forbidden());
    }
    let sms = match transport {
        "sms" if declaration.sms && declaration.address == AddressKind::E164Pair => true,
        "vapi-chat"
            if declaration.context
                && !declaration.sms
                && declaration.address == AddressKind::Uuid =>
        {
            false
        }
        "connector"
            if !declaration.sms
                && declaration.address != AddressKind::E164Pair
                && connect_protocol::provisioning::connector_endpoint_name(
                    target["address"].as_str().ok_or_else(Error::forbidden)?,
                )? == p.connector =>
        {
            false
        }
        _ => return Err(Error::forbidden()),
    };
    let target_address = target["address"].as_str().ok_or_else(Error::forbidden)?;
    let target_matches = if transport == "connector" {
        connect_protocol::provisioning::connector_endpoint_target(target_address)? == r.target
    } else {
        target_address == r.target
    };
    if target["identity"] != r.identity_id
        || !target_matches
        || target["transport"] != transport
        || target["owner"] != account.identity_id
        || target["secret_ref"] != p.secret_reference
        || !target["capabilities"]
            .as_array()
            .is_some_and(|v| v.iter().any(|x| x == "messaging"))
    {
        return Err(Error::forbidden());
    }
    let source = if sms {
        let source = endpoint(
            c,
            p.source_endpoint_id
                .as_deref()
                .ok_or_else(Error::forbidden)?,
        )?;
        if source["identity"] != account.identity_id
            || source["owner"] != account.identity_id
            || source["transport"] != "sms"
            || source["address"].as_str() != r.source.as_deref()
            || source["secret_ref"] != p.secret_reference
            || source["tenant"] != target["tenant"]
            || !source["capabilities"]
                .as_array()
                .is_some_and(|v| v.iter().any(|x| x == "messaging"))
        {
            return Err(Error::forbidden());
        }
        Some(source)
    } else {
        if p.source_endpoint_id.is_some() || r.source.is_some() {
            return Err(Error::forbidden());
        }
        let ai: bool = c
            .query_row(
                "SELECT kind='ai' FROM identities WHERE id=?1",
                [&r.identity_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        if transport == "vapi-chat" && !ai {
            return Err(Error::forbidden());
        }
        None
    };
    r.enabled = true;
    Ok(Facts {
        route: r,
        member,
        target,
        source,
        account_owner: account.identity_id,
    })
}
pub(crate) fn current(c: &Connection, r: &Route) -> Result<Option<ProviderMessageEndpoint>> {
    let old: Option<(String, String)> = c
        .query_row(
            "SELECT policy_json,source_json FROM provider_message_endpoints WHERE route_id=?1",
            [&r.id],
            |v| Ok((v.get(0)?, v.get(1)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let Some((policy, source)) = old else {
        return Ok(None);
    };
    let p: ProviderMessageEndpoint = serde_json::from_str(&policy).map_err(|_| Error::storage())?;
    let previous: Facts = serde_json::from_str(&source).map_err(|_| Error::storage())?;
    if !r.enabled || previous != facts(c, &p)? {
        return Err(Error::forbidden());
    }
    Ok(Some(p))
}
pub(crate) fn account(
    c: &Connection,
    p: &ProviderMessageEndpoint,
) -> Result<(i64, Vec<AuthorityScope>)> {
    let f = facts(c, p)?;
    let actor = actor_from_connection(c, &p.account_credential_id)?;
    Ok((
        actor.expires_at,
        vec![
            AuthorityScope::identity(&f.account_owner, AuthorityMode::Read)?,
            AuthorityScope::identity(&f.route.identity_id, AuthorityMode::Read)?,
        ],
    ))
}
pub(crate) fn configuration(
    c: &Connection,
    p: &ProviderMessageEndpoint,
    actual: &BTreeMap<String, String>,
) -> Result<()> {
    manifest(c, &p.connector)?
        .validate_account_reference(actual, &p.secret_reference)
        .map_err(|_| Error::forbidden())
}

impl Store {
    pub fn require_provider_message_configuration(
        &self,
        route_id: &str,
        configuration: &BTreeMap<String, String>,
    ) -> Result<()> {
        let c = self.connection()?;
        if let Some(p) = current(&c, &route(&c, route_id)?)? {
            self::configuration(&c, &p, configuration)?;
        }
        Ok(())
    }
    pub fn require_provider_observation_configuration(
        &self,
        job_id: &str,
        connector: &str,
        configuration: &BTreeMap<String, String>,
    ) -> Result<()> {
        let c = self.connection()?;
        let (job, _) = crate::connector_observations::authorized_job(&c, job_id, connector)?;
        if let Some(p) = current(&c, &job.route)? {
            self::configuration(&c, &p, configuration)?;
        }
        Ok(())
    }
    pub fn bind_provider_message_endpoints(
        &self,
        operator: &str,
        policies: &[ProviderMessageEndpoint],
    ) -> Result<()> {
        if policies.is_empty()
            || policies.len() > self.limits.connections.min(512)
            || policies
                .iter()
                .enumerate()
                .any(|(i, p)| policies[..i].iter().any(|old| old.route_id == p.route_id))
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid provider message route capacity",
            ));
        }
        let scopes = |c: &Connection| -> Result<Vec<AuthorityScope>> {
            let actor = actor_from_connection(c, operator)?;
            actor.require("admin")?;
            let mut scopes = vec![];
            for p in policies {
                let f = facts(c, p)?;
                if actor.identity_id != f.account_owner
                    || !actor.permits_recipient(&f.route.identity_id)
                {
                    return Err(Error::forbidden());
                }
                scopes.extend(crate::authority_admission::conversation_scopes(
                    c,
                    &f.route.conversation_id,
                    AuthorityMode::Write,
                )?);
                scopes.push(AuthorityScope::identity(
                    &f.account_owner,
                    AuthorityMode::Write,
                )?);
                scopes.push(AuthorityScope::identity(
                    &f.route.identity_id,
                    AuthorityMode::Write,
                )?);
            }
            Ok(scopes)
        };
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || scopes(&*self.connection()?),
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !self.authority_covers(&held, &scopes(&tx)?) {
            return Err(Error::forbidden());
        }
        for p in policies {
            let source = serde_json::to_string(&facts(&tx, p)?).map_err(|_| Error::storage())?;
            let policy = serde_json::to_string(p).map_err(|_| Error::storage())?;
            let previous:Option<(String,String)>=tx.query_row("SELECT policy_json,source_json FROM provider_message_endpoints WHERE route_id=?1",[&p.route_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
            match previous {
                Some(old) if old != (policy.clone(), source.clone()) => {
                    return Err(Error::new(
                        "conflict",
                        "Provider message endpoint binding is immutable",
                    ))
                }
                Some(_) => (),
                None => {
                    tx.execute(
                        "INSERT INTO provider_message_endpoints VALUES(?1,?2,?3)",
                        params![p.route_id, policy, source],
                    )
                    .map_err(|_| Error::storage())?;
                }
            }
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn provider_message_endpoint_current(&self, p: &ProviderMessageEndpoint) -> Result<bool> {
        let c = self.connection()?;
        Ok(current(&c, &route(&c, &p.route_id)?)?.as_ref() == Some(p))
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::Scope;
    use connect_protocol::{provisioning::*, Limits, Operation, Outcome};
    use std::sync::Arc;
    struct F {
        _tmp: tempfile::TempDir,
        s: Arc<Store>,
        root: String,
        root_id: String,
        ai: String,
        cid: String,
        p: ProviderMessageEndpoint,
    }
    fn ep(
        s: &Store,
        root: &str,
        owner: &str,
        name: &str,
        transport: EndpointTransport,
        address: &str,
    ) -> String {
        let ProvisioningOutcome::Endpoint { endpoint_id, .. } = s
            .execute_provisioning(
                root,
                ProvisioningCommand::Endpoint {
                    operation_id: format!("ep-{name}"),
                    identity_id: owner.into(),
                    device: DeviceSpec {
                        id: format!("device-{name}"),
                        kind: "service".into(),
                        platform: "fixture".into(),
                    },
                    transport,
                    address: address.into(),
                    capabilities: vec!["messaging".into()],
                    secret_ref: Some("MESSAGE_KEY".into()),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        s.execute_provisioning(
            root,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: format!("verify-{name}"),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "fixture".into(),
            },
        )
        .unwrap();
        endpoint_id
    }
    impl F {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let s = Arc::new(
                Store::initialize(&tmp.path().join("state.sqlite3"), Limits::default()).unwrap(),
            );
            for descriptor in [connect_connector_api::VAPI, connect_connector_api::TELNYX] {
                let mut declaration =
                    connect_connector_api::manifest::Manifest::messaging(descriptor);
                declaration
                    .configuration
                    .push(connect_connector_api::manifest::ConfigField {
                        name: if descriptor.sms {
                            "api-key"
                        } else {
                            "private-key"
                        }
                        .into(),
                        required: true,
                        secret: true,
                    });
                s.register_connector_manifest(descriptor, &declaration)
                    .unwrap();
            }
            let root_id = s.create_identity("root").unwrap().id;
            let root = s
                .issue(&root_id, Scope::operator(), crate::now() + 600)
                .unwrap()
                .id;
            let ProvisioningOutcome::Identity {
                identity_id: ai, ..
            } = s
                .execute_provisioning(
                    &root,
                    ProvisioningCommand::Identity {
                        operation_id: "ai".into(),
                        alias: "ai".into(),
                        kind: IdentityKind::Ai,
                        display_name: None,
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            let key = s
                .issue(&ai, Scope::operator(), crate::now() + 600)
                .unwrap()
                .id;
            let Outcome::Conversation(c) = s
                .execute(
                    &root,
                    &Operation::Create {
                        request_id: "room".into(),
                        request: connect_protocol::conversations::CreateRequest::persistent(vec![
                            ai.clone(),
                        ]),
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            s.configure_route(
                &root,
                &Route {
                    id: "bound".into(),
                    identity_id: ai.clone(),
                    credential_id: key.clone(),
                    conversation_id: c.id.clone(),
                    connector: "vapi".into(),
                    target: "12345678-1234-1234-1234-123456789012".into(),
                    source: None,
                    enabled: true,
                    incoming_to: connect_protocol::messages::MessageAudience::all(),
                },
            )
            .unwrap();
            let endpoint = ep(
                &s,
                &root,
                &ai,
                "target",
                EndpointTransport::VapiChat,
                "12345678-1234-1234-1234-123456789012",
            );
            let p = ProviderMessageEndpoint {
                route_id: "bound".into(),
                connector: "vapi".into(),
                transport: connect_protocol::provisioning::EndpointTransport::VapiChat,
                endpoint_id: endpoint,
                source_endpoint_id: None,
                account_credential_id: root.clone(),
                secret_reference: "MESSAGE_KEY".into(),
            };
            Self {
                _tmp: tmp,
                s,
                root,
                root_id,
                ai,
                cid: c.id,
                p,
            }
        }
        fn room(&self, name: &str) -> String {
            let Outcome::Conversation(c) = self
                .s
                .execute(
                    &self.root,
                    &Operation::Create {
                        request_id: name.into(),
                        request: connect_protocol::conversations::CreateRequest::persistent(vec![
                            self.ai.clone(),
                        ]),
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            c.id
        }
        fn bind(&self) {
            self.s
                .bind_provider_message_endpoints(&self.root, std::slice::from_ref(&self.p))
                .unwrap();
        }
        fn send(&self, id: &str) {
            self.s
                .execute(
                    &self.root,
                    &Operation::Send {
                        request_id: id.into(),
                        conversation_id: self.cid.clone(),
                        message: connect_protocol::messages::MessageDraft::text_all(id),
                    },
                )
                .unwrap();
        }
        fn count(&self, table: &str) -> u64 {
            self.s
                .connection()
                .unwrap()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }
    }
    #[test]
    fn binding_registration_is_atomic_exact_and_immutable() {
        let f = F::new();
        let mut bad = f.p.clone();
        bad.route_id = "missing".into();
        assert!(f
            .s
            .bind_provider_message_endpoints(&f.root, &[f.p.clone(), bad])
            .is_err());
        assert_eq!(f.count("provider_message_endpoints"), 0);
        f.bind();
        f.bind();
        assert_eq!(f.count("provider_message_endpoints"), 1);
        let mut changed = f.p.clone();
        changed.secret_reference = "OTHER_KEY".into();
        assert!(f
            .s
            .bind_provider_message_endpoints(&f.root, &[changed])
            .is_err());
        let c = f.s.connection().unwrap();
        assert!(c
            .execute("UPDATE provider_message_endpoints SET source_json='{}'", [])
            .is_err());
        assert!(c
            .execute("DELETE FROM provider_message_endpoints", [])
            .is_err());
    }
    #[test]
    fn selected_account_reference_is_required_before_single_use_transport_admission() {
        let f = F::new();
        f.bind();
        f.send("send");
        let role = f.s.acquire_provider_worker().unwrap();
        let job =
            f.s.claim_provider_job_for_routes(&["bound".into()])
                .unwrap()
                .unwrap();
        let wait = AuthorityWait::new(std::time::Duration::from_secs(1));
        assert!(f.s.admit_provider_dispatch(&role, &job, &wait).is_err());
        assert!(f
            .s
            .admit_configured_provider_dispatch(
                &role,
                &job,
                &wait,
                &BTreeMap::from([("private-key".into(), "secret:OTHER_KEY".into())])
            )
            .is_err());
        let permit =
            f.s.admit_configured_provider_dispatch(
                &role,
                &job,
                &wait,
                &BTreeMap::from([("private-key".into(), "secret:MESSAGE_KEY".into())]),
            )
            .unwrap();
        assert!(permit.check().is_ok());
        drop(permit);
        assert!(f
            .s
            .admit_configured_provider_dispatch(
                &role,
                &job,
                &wait,
                &BTreeMap::from([("private-key".into(), "secret:MESSAGE_KEY".into())])
            )
            .is_err());
    }
    #[test]
    fn withdrawn_endpoint_cancels_before_context_export_or_dispatch() {
        let f = F::new();
        f.bind();
        f.send("send");
        f.s.execute_provisioning(
            &f.root,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "withdraw".into(),
                endpoint_id: f.p.endpoint_id.clone(),
                expected_revision: 1,
            },
        )
        .unwrap();
        assert!(f
            .s
            .claim_provider_job_for_routes(&["bound".into()])
            .unwrap()
            .is_none());
        assert_eq!(f.count("context_deliveries"), 0);
        let state: String =
            f.s.connection()
                .unwrap()
                .query_row("SELECT state FROM provider_jobs", [], |r| r.get(0))
                .unwrap();
        assert_eq!(state, "cancelled");
    }
    #[test]
    fn endpoint_or_account_change_after_claim_refuses_before_io() {
        for revoke_account in [false, true] {
            let f = F::new();
            f.bind();
            f.send("send");
            let role = f.s.acquire_provider_worker().unwrap();
            let job =
                f.s.claim_provider_job_for_routes(&["bound".into()])
                    .unwrap()
                    .unwrap();
            if revoke_account {
                f.s.revoke(&f.s.revalidate(&f.root).unwrap(), &f.root)
                    .unwrap();
            } else {
                f.s.execute_provisioning(
                    &f.root,
                    ProvisioningCommand::RevokeEndpoint {
                        operation_id: "withdraw".into(),
                        endpoint_id: f.p.endpoint_id.clone(),
                        expected_revision: 1,
                    },
                )
                .unwrap();
            }
            assert!(f
                .s
                .admit_configured_provider_dispatch(
                    &role,
                    &job,
                    &AuthorityWait::new(std::time::Duration::from_secs(1)),
                    &BTreeMap::from([("private-key".into(), "secret:MESSAGE_KEY".into())])
                )
                .is_err());
            let admitted: bool =
                f.s.connection()
                    .unwrap()
                    .query_row(
                        "SELECT admitted FROM provider_dispatch WHERE job_id=?1",
                        [job.id],
                        |r| r.get(0),
                    )
                    .unwrap();
            assert!(!admitted);
        }
    }
    #[test]
    fn readmitted_identity_does_not_inherit_original_endpoint_route_epoch() {
        use connect_protocol::membership::*;
        let f = F::new();
        f.bind();
        let snapshot = f.s.membership_snapshot(&f.root, &f.cid).unwrap();
        let member = snapshot
            .participants
            .iter()
            .find(|p| p.identity_id == f.ai)
            .unwrap();
        f.s.execute(
            &f.root,
            &Operation::Membership {
                request: MembershipRequest {
                    operation_id: "remove".into(),
                    conversation_id: f.cid.clone(),
                    expected_revision: snapshot.revision,
                    change: MembershipAction::Remove {
                        participant_id: member.participant_id.clone(),
                        reason: None,
                    },
                },
            },
        )
        .unwrap();
        let snapshot = f.s.membership_snapshot(&f.root, &f.cid).unwrap();
        f.s.execute(
            &f.root,
            &Operation::Membership {
                request: MembershipRequest {
                    operation_id: "readmit".into(),
                    conversation_id: f.cid.clone(),
                    expected_revision: snapshot.revision,
                    change: MembershipAction::Add {
                        identity_id: f.ai.clone(),
                        kind: IdentityKind::Ai,
                        role: "participant".into(),
                    },
                },
            },
        )
        .unwrap();
        assert!(f.s.provider_message_endpoint_current(&f.p).is_err());
        assert!(f
            .s
            .bind_provider_message_endpoints(&f.root, &[f.p.clone()])
            .is_err());
    }
    #[test]
    fn scoped_claim_skips_older_unmounted_routes_without_consuming_their_jobs() {
        let f = F::new();
        f.bind();
        let mut other = route(&f.s.connection().unwrap(), "bound").unwrap();
        other.id = "earlier".into();
        other.conversation_id = f.room("unmounted-room");
        f.s.configure_route(&f.root, &other).unwrap();
        f.s.execute(
            &f.root,
            &Operation::Send {
                request_id: "unmounted-first".into(),
                conversation_id: other.conversation_id.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("unmounted-first"),
            },
        )
        .unwrap();
        f.send("mounted-second");
        let job =
            f.s.claim_provider_job_for_routes(&["bound".into()])
                .unwrap()
                .unwrap();
        assert_eq!(job.route.id, "bound");
        let n: u64 = f
            .s
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM provider_jobs WHERE route_id='earlier' AND state='pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
    #[test]
    fn sms_binding_requires_the_selected_account_owned_source_endpoint() {
        let f = F::new();
        let mut r = route(&f.s.connection().unwrap(), "bound").unwrap();
        r.id = "sms".into();
        r.conversation_id = f.room("sms-room");
        r.connector = "telnyx".into();
        r.target = "+15555550101".into();
        r.source = Some("+15555550102".into());
        f.s.configure_route(&f.root, &r).unwrap();
        let target = ep(
            &f.s,
            &f.root,
            &f.ai,
            "phone",
            EndpointTransport::Sms,
            &r.target,
        );
        let source = ep(
            &f.s,
            &f.root,
            &f.root_id,
            "source",
            EndpointTransport::Sms,
            r.source.as_deref().unwrap(),
        );
        let mut p = ProviderMessageEndpoint {
            route_id: "sms".into(),
            connector: "telnyx".into(),
            transport: connect_protocol::provisioning::EndpointTransport::Sms,
            endpoint_id: target,
            source_endpoint_id: None,
            account_credential_id: f.root.clone(),
            secret_reference: "MESSAGE_KEY".into(),
        };
        assert!(f
            .s
            .bind_provider_message_endpoints(&f.root, &[p.clone()])
            .is_err());
        p.source_endpoint_id = Some(source);
        f.s.bind_provider_message_endpoints(&f.root, &[p.clone()])
            .unwrap();
        assert!(f.s.provider_message_endpoint_current(&p).unwrap());
    }
    #[test]
    fn withdrawn_endpoint_refuses_identical_receipt_replay_before_effect_deduplication() {
        let f = F::new();
        f.bind();
        f.send("send");
        let job =
            f.s.claim_provider_job_for_routes(&["bound".into()])
                .unwrap()
                .unwrap();
        let receipt = connect_connector_api::lifecycle::message_attempt(
            &job.id,
            Ok(connect_connector_api::Completion {
                provider_id: "attributed-chat".into(),
                reply: None,
            }),
        );
        f.s.apply_connector_receipt("vapi", &receipt).unwrap();
        assert_eq!(f.count("connector_receipt_events"), 1);
        f.s.execute_provisioning(
            &f.root,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "withdraw".into(),
                endpoint_id: f.p.endpoint_id.clone(),
                expected_revision: 1,
            },
        )
        .unwrap();
        assert!(f.s.apply_connector_receipt("vapi", &receipt).is_err());
        assert_eq!(f.count("connector_receipt_events"), 1);
    }
    fn contributed(f: &F, name: &str, address: &str) -> ProviderMessageEndpoint {
        let descriptor = connect_connector_api::Descriptor {
            conversational: true,
            ..connect_connector_api::Descriptor::one_way("independent-fixture")
        };
        let mut declaration = connect_connector_api::manifest::Manifest::messaging(descriptor);
        declaration
            .configuration
            .push(connect_connector_api::manifest::ConfigField {
                name: "callback-key".into(),
                required: true,
                secret: true,
            });
        f.s.register_connector_manifest(descriptor, &declaration)
            .unwrap();
        let mut r = route(&f.s.connection().unwrap(), "bound").unwrap();
        let Outcome::Conversation(room) =
            f.s.execute(
                &f.root,
                &Operation::Create {
                    request_id: format!("room-{name}"),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![f
                        .ai
                        .clone()]),
                },
            )
            .unwrap()
        else {
            panic!("contributed room")
        };
        r.conversation_id = room.id;
        r.id = name.into();
        r.connector = "independent-fixture".into();
        r.target = connect_protocol::provisioning::connector_endpoint_target(address).unwrap();
        f.s.configure_route(&f.root, &r).unwrap();
        ProviderMessageEndpoint {
            route_id: name.into(),
            connector: "independent-fixture".into(),
            transport: EndpointTransport::Connector,
            endpoint_id: ep(
                &f.s,
                &f.root,
                &f.ai,
                name,
                EndpointTransport::Connector,
                address,
            ),
            source_endpoint_id: None,
            account_credential_id: f.root.clone(),
            secret_reference: "MESSAGE_KEY".into(),
        }
    }
    #[test]
    fn contributed_endpoint_binds_declared_account_and_rechecks_withdrawal() {
        let f = F::new();
        let p = contributed(&f, "custom", "connector://independent-fixture/peer");
        f.s.bind_provider_message_endpoints(&f.root, &[p.clone()])
            .unwrap();
        assert!(f.s.provider_message_endpoint_current(&p).unwrap());
        f.s.require_provider_message_configuration(
            &p.route_id,
            &[("callback-key".into(), "secret:MESSAGE_KEY".into())].into(),
        )
        .unwrap();
        assert!(f
            .s
            .require_provider_message_configuration(
                &p.route_id,
                &[("api-key".into(), "secret:MESSAGE_KEY".into())].into()
            )
            .is_err());
        assert!(f
            .s
            .require_provider_message_configuration(
                &p.route_id,
                &[("callback-key".into(), "secret:OTHER_KEY".into())].into()
            )
            .is_err());
        f.s.execute_provisioning(
            &f.root,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "withdraw-custom".into(),
                endpoint_id: p.endpoint_id.clone(),
                expected_revision: 1,
            },
        )
        .unwrap();
        assert!(f.s.provider_message_endpoint_current(&p).is_err());
        assert!(f
            .s
            .require_provider_message_configuration(
                &p.route_id,
                &[("callback-key".into(), "secret:MESSAGE_KEY".into())].into()
            )
            .is_err());
    }
    #[test]
    fn contributed_endpoint_refuses_wrong_declaration_transport_and_reference() {
        let f = F::new();
        let p = contributed(&f, "custom", "connector://independent-fixture/peer");
        for changed in [
            ProviderMessageEndpoint {
                transport: EndpointTransport::VapiChat,
                ..p.clone()
            },
            ProviderMessageEndpoint {
                secret_reference: "OTHER_KEY".into(),
                ..p.clone()
            },
            ProviderMessageEndpoint {
                connector: "vapi".into(),
                ..p.clone()
            },
        ] {
            assert!(f
                .s
                .bind_provider_message_endpoints(&f.root, &[changed])
                .is_err());
        }
        let other = contributed(&f, "wrong-name", "connector://other-fixture/peer");
        assert!(f
            .s
            .bind_provider_message_endpoints(&f.root, &[other])
            .is_err());
        f.s.bind_provider_message_endpoints(&f.root, &[p]).unwrap();
    }
}
