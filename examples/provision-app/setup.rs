//! Trusted backend only. All Connect access uses the public HTTP client.
use anyhow::{bail, ensure, Context, Result};
use connect_client::application::{
    credential_issuance::{CredentialIssueOutcome, CredentialIssueRequest},
    provisioning::{
        DeviceSpec, EndpointTransport, IdentityKind, ProvisioningCommand, ProvisioningOutcome,
        SigningAlgorithm,
    },
    ApplicationClient, ApplicationOperation,
};
use connect_protocol::{Operation, Outcome};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActorPlan {
    pub alias: String,
    pub display_name: String,
    pub device: DeviceSpec,
    pub transport: EndpointTransport,
    pub address: String,
    pub capabilities: Vec<String>,
    pub key_id: String,
    pub algorithm: SigningAlgorithm,
    /// Public verification material; private signing keys stay with their owner.
    pub public_key: String,
    /// Supplied by the trusted backend after its enrollment/ownership check.
    pub enrollment_evidence: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetupPlan {
    pub setup_id: String,
    /// Absolute, saved expiry; rerunning never extends it.
    pub not_before: i64,
    pub expires_at: i64,
    pub app: ActorPlan,
    pub human: ActorPlan,
    pub assistant: ActorPlan,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Request {
    Provision { command: ProvisioningCommand },
    Issue { request: CredentialIssueRequest },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    request: Request,
    outcome: Option<Value>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    profile: String,
    installation_id: String,
    plan: SetupPlan,
    steps: BTreeMap<String, Step>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contact {
    pub key: String,
    pub label: String,
    pub identity_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorEnrollment {
    pub identity_id: String,
    pub device_id: String,
    pub endpoint_id: String,
    pub key_id: String,
    pub grant: CredentialIssueOutcome,
}
/// Contains client credentials. Never serialize this into a web response or log.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedSetup {
    pub installation_id: String,
    pub actors: BTreeMap<String, ActorEnrollment>,
    pub contacts: Vec<Contact>,
}
struct Backend<'a> {
    client: &'a mut ApplicationClient,
    directory: PathBuf,
    journal: Journal,
    _lock: File,
}
const PROFILE: &str = "connect-provision-app/1";
// Explicit current controls for the example's three named actors; no wildcard,
// administrator, consent delegation, provider secret or arbitrary recipient.
fn operations() -> Vec<String> {
    [
        "health",
        "resolve",
        "create",
        "send",
        "history",
        "inbox",
        "request_status",
        "operation.get",
        "operation.cancel",
        "conversation.get",
        "conversation.list",
        "conversation.subscribe",
        "conversation.participant.add",
        "conversation.participant.update",
        "conversation.participant.remove",
        "session.observation",
        "session.invite",
        "session.accept",
        "session.reject",
        "session.cancel",
        "session.update",
        "session.end",
        "session.consult",
        "session.consult.return",
        "session.handoff",
        "connection.replace",
        "connection.offer",
        "connection.update",
        "connection.end",
        "stream.subscribe",
        "stream.unsubscribe",
        "capability.advertise",
        "dtmf.send",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
fn atomic_private(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .context("Private state requires a parent directory")?;
    let temporary = parent.join(format!(".setup-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "Setup journal exceeds its bound"
    );
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
fn bounded<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let info = fs::symlink_metadata(path)?;
    ensure!(
        info.is_file() && info.len() <= 1024 * 1024 && info.permissions().mode() & 0o077 == 0,
        "Setup state must be a bounded private regular file"
    );
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
impl Backend<'_> {
    fn save(&self) -> Result<()> {
        atomic_private(&self.directory.join("journal.json"), &self.journal)
    }
    async fn step(&mut self, name: &str, request: Request) -> Result<Value> {
        // Existing exact requests win over fresh resource observations. In
        // particular an Endpoint replay may now show its later verified revision.
        let request = if let Some(saved) = self.journal.steps.get(name) {
            saved.request.clone()
        } else {
            self.journal.steps.insert(
                name.into(),
                Step {
                    request: request.clone(),
                    outcome: None,
                },
            );
            self.save()?; // durable original intent BEFORE any mutation IO
            request
        };
        let value = match request {
            Request::Provision { command } => serde_json::to_value(
                self.client
                    .provision(command)
                    .await
                    .with_context(|| format!("Setup stage {name} was refused or unobserved"))?,
            )?,
            Request::Issue { request } => serde_json::to_value(
                self.client
                    .issue_scoped_credential(request)
                    .await
                    .with_context(|| format!("Setup stage {name} was refused or unobserved"))?,
            )?,
        };
        self.journal
            .steps
            .get_mut(name)
            .context("Original setup step missing")?
            .outcome = Some(value.clone());
        self.save()?;
        Ok(value)
    }
    async fn provision(
        &mut self,
        name: &str,
        mut command: ProvisioningCommand,
    ) -> Result<ProvisioningOutcome> {
        command.normalize()?;
        Ok(serde_json::from_value(
            self.step(name, Request::Provision { command }).await?,
        )?)
    }
}
/// Explicitly calling this again replays the saved original commands through
/// current server authorization. No transport failure is automatically retried.
pub async fn setup(
    client: &mut ApplicationClient,
    plan: SetupPlan,
    directory: &Path,
) -> Result<ProtectedSetup> {
    ensure!(
        !plan.setup_id.is_empty()
            && plan.setup_id.len() <= 48
            && plan
                .setup_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b)),
        "Use a stable bounded setup ID"
    );
    ensure!(
        plan.not_before <= chrono::Utc::now().timestamp()
            && plan.expires_at > chrono::Utc::now().timestamp()
            && plan.not_before < plan.expires_at,
        "Setup enrollment window is not current"
    );
    let specs = [
        ("app", IdentityKind::System, &plan.app),
        ("human", IdentityKind::Human, &plan.human),
        ("assistant", IdentityKind::Ai, &plan.assistant),
    ];
    let mut aliases = std::collections::BTreeSet::new();
    let mut devices = std::collections::BTreeSet::new();
    let mut keys = std::collections::BTreeSet::new();
    let mut public_keys = std::collections::BTreeSet::new();
    for (key, kind, actor) in &specs {
        ensure!(
            aliases.insert(&actor.alias)
                && devices.insert(&actor.device.id)
                && keys.insert(&actor.key_id)
                && public_keys.insert(&actor.public_key),
            "Actors require distinct aliases, devices and public keys"
        );
        ensure!(
            matches!(
                actor.transport,
                EndpointTransport::Quic
                    | EndpointTransport::Webtransport
                    | EndpointTransport::Webrtc
            ),
            "This example provisions only explicitly owned native/browser endpoints"
        );
        // Check the complete public shape before reserving any Identity.
        // Placeholder IDs are syntax-only; actual IDs come from public receipts.
        let operation_id = format!("{}-{key}-shape", plan.setup_id);
        for mut command in [
            ProvisioningCommand::Identity {
                operation_id: operation_id.clone(),
                alias: actor.alias.clone(),
                kind: kind.clone(),
                display_name: Some(actor.display_name.clone()),
            },
            ProvisioningCommand::Endpoint {
                operation_id: operation_id.clone(),
                identity_id: "pending".into(),
                device: actor.device.clone(),
                transport: actor.transport.clone(),
                address: actor.address.clone(),
                capabilities: actor.capabilities.clone(),
                secret_ref: None,
            },
            ProvisioningCommand::VerifyEndpoint {
                operation_id: operation_id.clone(),
                endpoint_id: "pending".into(),
                expected_revision: 1,
                evidence_ref: actor.enrollment_evidence.clone(),
            },
            ProvisioningCommand::SigningKey {
                operation_id,
                identity_id: "pending".into(),
                credential_id: "pending".into(),
                device_id: actor.device.id.clone(),
                key_id: actor.key_id.clone(),
                algorithm: actor.algorithm,
                public_key: actor.public_key.clone(),
                not_before: plan.not_before,
                expires_at: plan.expires_at,
                expected_revision: None,
            },
        ] {
            command.normalize()?;
        }
    }
    if !directory.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    let info = fs::symlink_metadata(directory)?;
    ensure!(
        info.is_dir() && info.permissions().mode() & 0o077 == 0,
        "Choose a private setup directory"
    );
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join("setup.lock"))?;
    lock.try_lock_exclusive()
        .context("This setup is already running")?;
    let installation = match client
        .execute(ApplicationOperation::try_from(Operation::Health {})?)
        .await?
    {
        Outcome::Health {
            installation_id, ..
        } => installation_id,
        _ => bail!("Expected installation health"),
    };
    let journal_path = directory.join("journal.json");
    let journal = if journal_path.exists() {
        let saved: Journal = bounded(&journal_path)?;
        ensure!(
            saved.profile == PROFILE && saved.installation_id == installation && saved.plan == plan,
            "Saved setup belongs to a different installation or immutable plan"
        );
        saved
    } else {
        Journal {
            profile: PROFILE.into(),
            installation_id: installation.clone(),
            plan: plan.clone(),
            steps: BTreeMap::new(),
        }
    };
    let mut backend = Backend {
        client,
        directory: directory.into(),
        journal,
        _lock: lock,
    };
    backend.save()?;
    let id = |actor: &str, action: &str| format!("{}-{actor}-{action}", plan.setup_id);
    let mut identities = BTreeMap::new();
    for (key, kind, actor) in &specs {
        let result = backend
            .provision(
                &format!("{key}.identity"),
                ProvisioningCommand::Identity {
                    operation_id: id(key, "identity"),
                    alias: actor.alias.clone(),
                    kind: kind.clone(),
                    display_name: Some(actor.display_name.clone()),
                },
            )
            .await?;
        let ProvisioningOutcome::Identity {
            identity_id,
            revoked: false,
            ..
        } = result
        else {
            bail!("Identity is not current");
        };
        identities.insert(*key, identity_id);
    }
    let mut actors = BTreeMap::new();
    let mut contacts = Vec::new();
    for (key, _, actor) in specs {
        let identity = identities[key].clone();
        let result = backend
            .provision(
                &format!("{key}.endpoint"),
                ProvisioningCommand::Endpoint {
                    operation_id: id(key, "endpoint"),
                    identity_id: identity.clone(),
                    device: actor.device.clone(),
                    transport: actor.transport.clone(),
                    address: actor.address.clone(),
                    capabilities: actor.capabilities.clone(),
                    secret_ref: None,
                },
            )
            .await?;
        let ProvisioningOutcome::Endpoint {
            endpoint_id,
            revision,
            ref verification_state,
            ..
        } = result
        else {
            bail!("Endpoint is not current");
        };
        ensure!(verification_state != "revoked", "Endpoint is revoked");
        let verified = backend
            .provision(
                &format!("{key}.verify"),
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: id(key, "verify"),
                    endpoint_id: endpoint_id.clone(),
                    expected_revision: revision,
                    evidence_ref: actor.enrollment_evidence.clone(),
                },
            )
            .await?;
        ensure!(
            matches!(verified, ProvisioningOutcome::Endpoint { ref verification_state,.. } if verification_state=="verified"),
            "Endpoint ownership is not verified"
        );
        let mut request = CredentialIssueRequest {
            operation_id: id(key, "credential"),
            identity_id: identity.clone(),
            operations: operations(),
            recipients: identities
                .values()
                .filter(|other| **other != identity)
                .cloned()
                .collect(),
            expires_at: plan.expires_at,
        };
        request.normalize()?;
        let grant: CredentialIssueOutcome = serde_json::from_value(
            backend
                .step(&format!("{key}.credential"), Request::Issue { request })
                .await?,
        )?;
        let signing = backend
            .provision(
                &format!("{key}.key"),
                ProvisioningCommand::SigningKey {
                    operation_id: id(key, "key"),
                    identity_id: identity.clone(),
                    credential_id: grant.credential_id.clone(),
                    device_id: actor.device.id.clone(),
                    key_id: actor.key_id.clone(),
                    algorithm: actor.algorithm.clone(),
                    public_key: actor.public_key.clone(),
                    not_before: plan.not_before,
                    expires_at: plan.expires_at,
                    expected_revision: None,
                },
            )
            .await?;
        ensure!(
            matches!(
                signing,
                ProvisioningOutcome::SigningKey { revoked: false, .. }
            ),
            "Signing key is not current"
        );
        actors.insert(
            key.into(),
            ActorEnrollment {
                identity_id: identity.clone(),
                device_id: actor.device.id.clone(),
                endpoint_id,
                key_id: actor.key_id.clone(),
                grant,
            },
        );
        contacts.push(Contact {
            key: key.into(),
            label: actor.display_name.clone(),
            identity_id: identity,
        });
    }
    let result = ProtectedSetup {
        installation_id: installation,
        actors,
        contacts,
    };
    atomic_private(&directory.join("enrollments.private.json"), &result)?;
    atomic_private(&directory.join("contacts.json"), &result.contacts)?;
    Ok(result)
}
