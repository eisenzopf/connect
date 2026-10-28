//! Explicit trusted backend adapter; participant helper actions cannot reach it.
use super::*;
pub const PROFILE: &str = "connect-sdk-administration/2";
pub const VALIDATION_PROFILE: &str = "connect-sdk-provisioning-validation/1";

pub fn validate_bytes(bytes: &[u8], reference: bool) -> Value {
    if bytes.len() > if reference { 4096 } else { 65536 } {
        return json!({"profile":VALIDATION_PROFILE,"valid":false});
    }
    let result = serde_json::from_slice(bytes)
        .map_err(|_| Error::local(ErrorKind::Validation))
        .and_then(|value| validate(value, reference));
    match result {
        Ok(mut value) => {
            value["profile"] = json!(VALIDATION_PROFILE);
            value
        }
        Err(_) => json!({"profile":VALIDATION_PROFILE,"valid":false}),
    }
}
fn validate(value: Value, reference: bool) -> Result<Value> {
    if !reference {
        let command = TrustedProvisioningCommand::parse(value)?;
        return Ok(
            json!({"valid":true,"value":command.to_json(),"reference":command.reference_json(),"kind":"administrative"}),
        );
    }
    let command = value["command"]
        .as_str()
        .ok_or_else(|| Error::local(ErrorKind::Validation))?;
    let (saved, kind) = match command {
        "operation.cancel" => (
            OperationReference::parse(value, OperationKinds::new().operation_cancel)?.to_json(),
            "operation_cancel",
        ),
        "operation.reconcile" => (
            OperationReference::parse(value, OperationKinds::new().operation_reconcile)?.to_json(),
            "operation_reconcile",
        ),
        command => {
            let kind = super::super::administration::kind(command)?;
            (
                OperationReference::parse(value, kind)?.to_json(),
                "administrative",
            )
        }
    };
    if saved["scope"] != json!({}) || saved.get("continuation").is_some() {
        return Err(Error::local(ErrorKind::Validation));
    }
    Ok(json!({"valid":true,"value":saved,"reference":saved,"kind":kind}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub timeout_ms: u64,
    pub action: Action,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    AdministrativeProvision {
        command: Value,
        mutation: Mutation,
    },
    AdministrativeOperation {
        reference: Value,
        action: OperationAction,
    },
}
pub async fn execute(client: &AdministrativeClient, request: Request) -> Result<Value> {
    let options = RequestOptions::new(Duration::from_millis(request.timeout_ms))?;
    match request.action {
        Action::AdministrativeProvision { command, mutation } => Ok(client
            .provision(
                client.validate_provisioning(command)?,
                mutation.options(options),
            )
            .await?
            .reference_json()),
        Action::AdministrativeOperation { reference, action } => {
            validate(reference.clone(), true)?;
            match reference["command"].as_str() {
                Some("operation.cancel") => {
                    super::run(
                        client
                            .control()
                            .restore(reference, OperationKinds::new().operation_cancel)?,
                        action,
                        options,
                        super::decision_json,
                    )
                    .await
                }
                Some("operation.reconcile") => {
                    super::run(
                        client
                            .control()
                            .restore(reference, OperationKinds::new().operation_reconcile)?,
                        action,
                        options,
                        super::decision_json,
                    )
                    .await
                }
                _ => super::run(client.restore(reference)?, action, options, |value| value).await,
            }
        }
    }
}
