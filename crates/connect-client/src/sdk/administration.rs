//! Trusted provisioning owns bounded HTTP work and original-ID native status.
//!
//! ```no_run
//! # async fn provision(control: connect_client::canonical::CanonicalClientConfig,
//! # application: connect_client::application::ApplicationClientConfig,
//! # saved_id: connect_client::sdk::OperationId) -> connect_client::sdk::Result<()> {
//! use connect_client::sdk::{AdministrativeClient, MutationOptions, RequestOptions};
//! use std::time::Duration;
//! let admin = AdministrativeClient::connect(control, "tnt_default".parse()?, application).await?;
//! let wait = RequestOptions::new(Duration::from_secs(10))?;
//! let intent = admin.validate_provisioning(serde_json::json!({
//!     "type":"identity", "operation_id":saved_id, "alias":"backend-target",
//!     "kind":"system", "display_name":null
//! }))?;
//! let saved = intent.reference_json();
//! let original = intent.to_json(); // Durably persist both before IO.
//! let operation = admin.provision(intent, MutationOptions {
//!     operation_id:saved_id, request:wait.clone(), extensions:None,
//! }).await?;
//! operation.wait(wait.clone()).await?;
//! admin.restore(saved)?.status(wait.clone()).await?; // Read only.
//! let _ = original;
//! admin.close(wait).await?;
//! # Ok(()) }
//! ```
use super::*;
use crate::application::{ApplicationClient, ApplicationClientConfig};
use crate::canonical::CanonicalClientConfig;
use connect_protocol::provisioning::{ProvisioningCommand, ProvisioningOutcome};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot, Mutex, OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub struct TrustedProvisioningCommand {
    command: ProvisioningCommand,
    id: OperationId,
}
impl std::fmt::Debug for TrustedProvisioningCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedProvisioningCommand")
            .field("command", &self.command.command_type())
            .field("operation_id", &self.id)
            .finish_non_exhaustive()
    }
}
impl TrustedProvisioningCommand {
    pub fn operation_id(&self) -> &OperationId {
        &self.id
    }
    /// Persist before submission together with to_json(). Restoring this
    /// reference is a current-authorized status read, never a resubmission.
    pub fn reference_json(&self) -> Value {
        serde_json::json!({"schema":"connect-operation-reference/1","operation_id":self.id,"command":self.command.command_type(),"scope":{}})
    }
    /// Explicit durable intent export. Debug output never includes this body.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(&self.command).expect("validated provisioning command")
    }
    fn parse(value: Value) -> Result<Self> {
        if serde_json::to_vec(&value)
            .map_err(|_| Error::local(ErrorKind::Validation))?
            .len()
            > 65_536
        {
            return Err(Error::local(ErrorKind::Validation));
        }
        let mut command: ProvisioningCommand =
            serde_json::from_value(value).map_err(|_| Error::local(ErrorKind::Validation))?;
        command
            .normalize()
            .map_err(|_| Error::local(ErrorKind::Validation))?;
        // These shared IDs would be normalized away by HTTP path parsing.
        let value =
            serde_json::to_value(&command).map_err(|_| Error::local(ErrorKind::Validation))?;
        if serde_json::to_vec(&value)
            .map_err(|_| Error::local(ErrorKind::Validation))?
            .len()
            > 65_536
        {
            return Err(Error::local(ErrorKind::Validation));
        }
        for name in [
            "conversation_id",
            "identity_id",
            "endpoint_id",
            "credential_id",
            "device_id",
            "key_id",
        ] {
            if value[name]
                .as_str()
                .is_some_and(|s| matches!(s, "." | ".."))
            {
                return Err(Error::local(ErrorKind::Validation));
            }
        }
        let id = command.operation_id().parse()?;
        Ok(Self { command, id })
    }
}
pub(super) fn is_command(command: &str) -> bool {
    kind(command).is_ok()
}
fn unscoped(scope: &OperationScope) -> bool {
    scope.conversation_id.is_none() && scope.session_id.is_none() && scope.connection_id.is_none()
}
fn kind(command: &str) -> Result<OperationKind<Value>> {
    let command = match command {
        "record-read-role.configure" => "record-read-role.configure",
        "identity.provision" => "identity.provision",
        "participant.provision" => "participant.provision",
        "endpoint.provision" => "endpoint.provision",
        "endpoint.verify" => "endpoint.verify",
        "endpoint.revoke" => "endpoint.revoke",
        "identity.revoke" => "identity.revoke",
        "signing-key.enroll" => "signing-key.enroll",
        "signing-key.revoke" => "signing-key.revoke",
        "device.revoke" => "device.revoke",
        "credential.revoke" => "credential.revoke",
        _ => return Err(Error::local(ErrorKind::Validation)),
    };
    Ok(OperationKind::new(command, decode, unscoped))
}
fn decode(value: Value, _client: &Client, key: &OperationKey) -> Result<Value> {
    let bad = || Error::request(ErrorKind::Protocol, Some(key), false);
    let parsed: ProvisioningOutcome = serde_json::from_value(value.clone()).map_err(|_| bad())?;
    let matches = match (key.command.as_str(), parsed) {
        ("record-read-role.configure", ProvisioningOutcome::RecordReadRole { .. })
        | ("identity.provision" | "identity.revoke", ProvisioningOutcome::Identity { .. })
        | ("participant.provision", ProvisioningOutcome::Participant { .. })
        | (
            "endpoint.provision" | "endpoint.verify" | "endpoint.revoke",
            ProvisioningOutcome::Endpoint { .. },
        )
        | ("signing-key.enroll" | "signing-key.revoke", ProvisioningOutcome::SigningKey { .. })
        | ("device.revoke", ProvisioningOutcome::Device { .. })
        | ("credential.revoke", ProvisioningOutcome::Credential { .. }) => true,
        _ => false,
    };
    if !matches || !unscoped(&key.scope) {
        return Err(bad());
    }
    for field in [
        "identity_id",
        "credential_id",
        "key_id",
        "device_id",
        "endpoint_id",
        "participant_id",
        "conversation_id",
        "tenant_id",
    ] {
        if let Some(v) = value.get(field) {
            rvoip_uctp::v2::Identifier::try_from(v.as_str().ok_or_else(bad)?.to_owned())
                .map_err(|_| bad())?;
        }
    }
    for field in ["revision", "membership_epoch"] {
        if let Some(v) = value.get(field) {
            let n = v
                .as_u64()
                .filter(|n| *n <= 9_007_199_254_740_991)
                .ok_or_else(bad)?;
            if field == "membership_epoch" && n == 0 {
                return Err(bad());
            }
        }
    }
    Ok(value)
}
fn application_error(error: crate::ClientError, key: &OperationKey) -> Error {
    if let crate::ClientError::Application(problem) = error {
        let problem = match problem.code.as_str() {
            "stale-revision" => connect_protocol::canonical_http::Problem {
                code: 409,
                category: "conflict".into(),
                reason: "revision_conflict".into(),
            },
            "unsupported-profile" => connect_protocol::canonical_http::Problem {
                code: 501,
                category: "unsupported".into(),
                reason: "unsupported_profile".into(),
            },
            "payload-too-large" => connect_protocol::canonical_http::Problem {
                code: 413,
                category: "validation".into(),
                reason: "request_too_large".into(),
            },
            "operation-pending" => connect_protocol::canonical_http::Problem {
                code: 503,
                category: "state".into(),
                reason: "operation_pending".into(),
            },
            _ => connect_protocol::canonical_http::Problem::from_error(&problem),
        };
        let kind = match problem.code {
            401 | 403 => ErrorKind::Authentication,
            409 => ErrorKind::Conflict,
            404 | 410 => ErrorKind::ReceiptUnavailable,
            400 | 413 => ErrorKind::Validation,
            501 => ErrorKind::Unsupported,
            _ => ErrorKind::Application,
        };
        let certainty = if problem.code >= 500 && problem.code != 501 {
            Certainty::Uncertain
        } else {
            Certainty::Refused
        };
        Error {
            kind,
            certainty,
            operation: Some(key.clone()),
            problem: Some(CanonicalProblem {
                code: problem.code.into(),
                category: problem.category,
                reason: problem.reason,
            }),
            observation: None,
        }
    } else {
        Error::transport(error, Some(key))
    }
}
struct Submission {
    command: ProvisioningCommand,
    key: OperationKey,
    expires: tokio::time::Instant,
    signal: CancellationToken,
    reply: oneshot::Sender<Result<ProvisioningOutcome>>,
    _slot: OwnedSemaphorePermit,
}
struct OwnedApplication {
    requests: mpsc::Sender<Submission>,
    slots: Arc<Semaphore>,
    stop: CancellationToken,
    closed: AtomicBool,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    native_closed: Arc<AtomicBool>,
}
impl Drop for OwnedApplication {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl OwnedApplication {
    fn start(mut application: ApplicationClient, native_closed: Arc<AtomicBool>) -> Arc<Self> {
        let (requests, mut receive) = mpsc::channel::<Submission>(16);
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        let slots = Arc::new(Semaphore::new(16));
        let native = native_closed.clone();
        let task = tokio::spawn(async move {
            loop {
                let request = tokio::select! {biased;_=stopping.cancelled()=>break,request=receive.recv()=>match request {Some(request)=>request,None=>break}};
                if request.reply.is_closed()
                    || request.signal.is_cancelled()
                    || tokio::time::Instant::now() >= request.expires
                    || native.load(Ordering::Acquire)
                {
                    let kind = if tokio::time::Instant::now() >= request.expires {
                        ErrorKind::Deadline
                    } else {
                        ErrorKind::Cancelled
                    };
                    let _ =
                        request
                            .reply
                            .send(Err(Error::request(kind, Some(&request.key), false)));
                    continue;
                }
                // Only closing this owned client cancels started HTTP IO. A caller losing
                // its wait never abandons this exchange or changes its original operation ID.
                let result = application
                    .provision_with_cancel(request.command, stopping.clone())
                    .await
                    .map_err(|e| application_error(e, &request.key));
                let _ = request.reply.send(result);
            }
            receive.close();
            while let Some(request) = receive.recv().await {
                let _ = request.reply.send(Err(Error::request(
                    ErrorKind::Cancelled,
                    Some(&request.key),
                    false,
                )));
            }
        });
        Arc::new(Self {
            requests,
            slots,
            stop,
            closed: AtomicBool::new(false),
            task: Mutex::new(Some(task)),
            native_closed,
        })
    }
    async fn submit(
        &self,
        command: ProvisioningCommand,
        key: &OperationKey,
        options: RequestOptions,
    ) -> Result<ProvisioningOutcome> {
        options.validate()?;
        if self.closed.load(Ordering::Acquire)
            || self.native_closed.load(Ordering::Acquire)
            || options.signal.is_cancelled()
        {
            return Err(Error::request(ErrorKind::Cancelled, Some(key), false));
        }
        let expires = tokio::time::Instant::now() + options.timeout;
        let slot = tokio::select! {biased;_=options.signal.cancelled()=>return Err(Error::request(ErrorKind::Cancelled,Some(key),false)),result=tokio::time::timeout_at(expires,self.slots.clone().acquire_owned())=>result.map_err(|_|Error::request(ErrorKind::Deadline,Some(key),false))?.map_err(|_|Error::request(ErrorKind::Cancelled,Some(key),false))?};
        let permit = tokio::select! {biased;_=options.signal.cancelled()=>return Err(Error::request(ErrorKind::Cancelled,Some(key),false)),result=tokio::time::timeout_at(expires,self.requests.reserve())=>result.map_err(|_|Error::request(ErrorKind::Deadline,Some(key),false))?.map_err(|_|Error::request(ErrorKind::Cancelled,Some(key),false))?};
        let (reply, response) = oneshot::channel();
        permit.send(Submission {
            command,
            key: key.clone(),
            expires,
            signal: options.signal.clone(),
            reply,
            _slot: slot,
        });
        tokio::select! {biased;_=options.signal.cancelled()=>Err(Error::request(ErrorKind::Cancelled,Some(key),true)),result=tokio::time::timeout_at(expires,response)=>result.map_err(|_|Error::request(ErrorKind::Deadline,Some(key),true))?.map_err(|_|Error::request(ErrorKind::TransportUncertain,Some(key),true))?}
    }
    async fn close(&self, options: RequestOptions) -> Result<()> {
        options.validate()?;
        self.closed.store(true, Ordering::Release);
        self.stop.cancel();
        self.slots.close();
        let until = tokio::time::Instant::now() + options.timeout;
        let mut task = tokio::select! {biased;_=options.signal.cancelled()=>return Err(Error::local(ErrorKind::CleanupIncomplete)),result=tokio::time::timeout_at(until,self.task.lock())=>result.map_err(|_|Error::local(ErrorKind::CleanupIncomplete))?};
        if let Some(job) = task.as_mut() {
            let result = tokio::select! {biased;_=options.signal.cancelled()=>return Err(Error::local(ErrorKind::CleanupIncomplete)),result=tokio::time::timeout_at(until,job)=>result};
            match result {
                Ok(result) => {
                    *task = None;
                    result.map_err(|_| Error::local(ErrorKind::CleanupIncomplete))?;
                }
                Err(_) => return Err(Error::local(ErrorKind::CleanupIncomplete)),
            }
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct AdministrativeClient {
    control: Client,
    application: Arc<OwnedApplication>,
}
impl std::fmt::Debug for AdministrativeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdministrativeClient")
            .finish_non_exhaustive()
    }
}
impl AdministrativeClient {
    /// Explicit local application HTTP provisioning and signed native status must
    /// use the same supplied credential. No endpoint or credential fallback exists.
    pub async fn connect(
        control: CanonicalClientConfig,
        tenant_id: TenantId,
        application: ApplicationClientConfig,
    ) -> Result<Self> {
        control.validate().map_err(|e| Error::transport(e, None))?;
        application
            .validate()
            .map_err(|e| Error::transport(e, None))?;
        if control.credential != application.credential {
            return Err(Error::local(ErrorKind::Configuration));
        }
        let application = ApplicationClient::open(application)
            .await
            .map_err(|e| Error::transport(e, None))?;
        let control = Client::connect(control, tenant_id).await?;
        Ok(Self::from_parts(control, application))
    }
    pub(crate) fn from_parts(control: Client, application: ApplicationClient) -> Self {
        let application = OwnedApplication::start(application, control.shared.closed.clone());
        Self {
            control,
            application,
        }
    }
    pub fn validate_provisioning(&self, value: Value) -> Result<TrustedProvisioningCommand> {
        TrustedProvisioningCommand::parse(value)
    }
    pub fn restore(&self, saved: Value) -> Result<Operation<Value>> {
        let command = saved["command"]
            .as_str()
            .ok_or_else(|| Error::local(ErrorKind::Validation))?;
        let kind = kind(command)?;
        self.control.restore(saved, kind)
    }
    pub async fn provision(
        &self,
        command: TrustedProvisioningCommand,
        options: MutationOptions,
    ) -> Result<Operation<Value>> {
        let kind = kind(command.command.command_type())?;
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope: OperationScope::default(),
        };
        if command.id != options.operation_id {
            return Err(Error::request(ErrorKind::Validation, Some(&key), false));
        }
        if options.extensions.as_ref().is_some_and(|e| !e.is_empty()) {
            return Err(Error::request(ErrorKind::Unsupported, Some(&key), false));
        }
        let outcome = self
            .application
            .submit(command.command, &key, options.request)
            .await?;
        let value = serde_json::to_value(outcome)
            .map_err(|_| Error::request(ErrorKind::Protocol, Some(&key), true))?;
        (kind.decode)(value, &self.control, &key).map_err(|mut e| {
            e.certainty = Certainty::Uncertain;
            e
        })?;
        Ok(Operation::new(
            self.control.clone(),
            OperationReference {
                key,
                kind,
                continuation: None,
            },
        ))
    }
    pub async fn close(&self, options: RequestOptions) -> Result<()> {
        let (application, control) = tokio::join!(
            self.application.close(options.clone()),
            self.control.close(options)
        );
        application?;
        control
    }
}
