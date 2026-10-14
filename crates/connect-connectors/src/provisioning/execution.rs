//! Checkpoint before provider attempts. Unknown outcomes reconcile before any retry.
use super::{Intent, Selection};
use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub resource_id: String,
    pub created: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "receipt", rename_all = "snake_case")]
pub enum StepState {
    Prepared,
    Running,
    Accepted(Receipt),
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    pub mutation_id: String,
    pub selection: Selection,
    pub state: StepState,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    pub profile: String,
    pub intent: Intent,
    pub operation_id: String,
    pub steps: Vec<Step>,
    pub cancelled: bool,
}
/// A positive absence requires authoritative reconciliation, never a timeout.
pub enum Reconciliation {
    Accepted(Receipt),
    NotAccepted,
    Unknown,
}
pub enum Attempt {
    Accepted(Receipt),
    Rejected,
    Unknown,
}
/// An independently supplied provisioning adapter owns provider protocol details.
#[async_trait]
pub trait Provisioner: Send + Sync {
    async fn attempt(&self, mutation_id: &str, selection: &Selection) -> Result<Attempt>;
    async fn reconcile(&self, mutation_id: &str, selection: &Selection) -> Result<Reconciliation>;
}
fn validate_receipt(selection: &Selection, receipt: &Receipt) -> Result<()> {
    ensure!(
        !receipt.resource_id.is_empty() && receipt.resource_id.len() <= 256,
        "Accepted resource has no usable ID; preserve running checkpoint"
    );
    match selection {
        Selection::Reuse { id, .. } => ensure!(
            receipt.resource_id == *id && !receipt.created,
            "Reuse receipt cannot claim creation or another resource"
        ),
        Selection::Create { .. } => ensure!(
            receipt.created,
            "Creation needs an explicit ownership receipt"
        ),
    }
    Ok(())
}
fn write(path: &Path, state: &Execution) -> Result<()> {
    let tmp = path.with_extension(format!("{}.pending", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(state)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
fn lock(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path.with_extension("lock"))?;
    file.try_lock_exclusive()
        .context("Another setup execution owns this state")?;
    Ok(file)
}
/// Inspect exact persisted intent and owned receipts; no provider request occurs.
pub fn inspect_execution(path: &Path) -> Result<Execution> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "Setup checkpoint must be a regular private file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "Setup checkpoint requires mode 0600"
        );
    }
    ensure!(meta.len() <= 1024 * 1024, "Setup checkpoint exceeds limit");
    let state: Execution = serde_json::from_slice(&fs::read(path)?)?;
    state.intent.validate()?;
    ensure!(
        state.profile == "connect-setup-execution/1"
            && state.operation_id.len() == 32
            && state.operation_id.bytes().all(|c| c.is_ascii_hexdigit())
            && state.steps.len() == state.intent.selections.len(),
        "Invalid setup checkpoint"
    );
    for (index, step) in state.steps.iter().enumerate() {
        ensure!(
            step.selection == state.intent.selections[index]
                && step.mutation_id == format!("{}-{index}", state.operation_id),
            "Checkpoint no longer matches approved intent"
        );
        if let StepState::Accepted(receipt) = &step.state {
            validate_receipt(&step.selection, receipt)?;
        }
    }
    Ok(state)
}
/// Execute approved intent using stable per-step IDs. A crash during attempt leaves
/// Running; the next invocation must reconcile it before sending another request.
pub async fn execute(path: &Path, intent: &Intent, adapter: &dyn Provisioner) -> Result<Execution> {
    intent.validate()?;
    let _lock = lock(path)?;
    let mut state = if path.exists() {
        let state = inspect_execution(path)?;
        ensure!(
            state.intent == *intent,
            "Approved setup intent changed; use a separate state path"
        );
        state
    } else {
        let operation_id = uuid::Uuid::new_v4().simple().to_string();
        let steps = intent
            .selections
            .iter()
            .enumerate()
            .map(|(index, selection)| Step {
                mutation_id: format!("{operation_id}-{index}"),
                selection: selection.clone(),
                state: StepState::Prepared,
            })
            .collect();
        let state = Execution {
            profile: "connect-setup-execution/1".into(),
            intent: intent.clone(),
            operation_id,
            steps,
            cancelled: false,
        };
        write(path, &state)?;
        state
    };
    if state.cancelled {
        return Ok(state);
    }
    for index in 0..state.steps.len() {
        if matches!(state.steps[index].state, StepState::Accepted(_)) {
            continue;
        }
        if matches!(
            state.steps[index].state,
            StepState::Failed | StepState::Cancelled
        ) {
            return Ok(state);
        }
        if state.steps[index].state == StepState::Running {
            let step = &state.steps[index];
            match adapter
                .reconcile(&step.mutation_id, &step.selection)
                .await?
            {
                Reconciliation::Unknown => return Ok(state),
                Reconciliation::Accepted(receipt) => {
                    validate_receipt(&step.selection, &receipt)?;
                    state.steps[index].state = StepState::Accepted(receipt);
                    write(path, &state)?;
                    continue;
                }
                Reconciliation::NotAccepted => {
                    state.steps[index].state = StepState::Prepared;
                    write(path, &state)?;
                }
            }
        }
        state.steps[index].state = StepState::Running;
        write(path, &state)?;
        let step = &state.steps[index];
        match adapter.attempt(&step.mutation_id, &step.selection).await? {
            Attempt::Accepted(receipt) => {
                validate_receipt(&step.selection, &receipt)?;
                state.steps[index].state = StepState::Accepted(receipt);
            }
            Attempt::Rejected => state.steps[index].state = StepState::Failed,
            Attempt::Unknown => return Ok(state),
        }
        write(path, &state)?;
        if state.steps[index].state == StepState::Failed {
            break;
        }
    }
    Ok(state)
}
/// Cancellation stops future requests. Running steps remain uncertain; no resource
/// is deleted or rollback inferred. Cleanup requires reconciled owned receipts.
pub fn cancel(path: &Path) -> Result<Execution> {
    let _lock = lock(path)?;
    let mut state = inspect_execution(path)?;
    state.cancelled = true;
    for step in &mut state.steps {
        if step.state == StepState::Prepared {
            step.state = StepState::Cancelled;
        }
    }
    write(path, &state)?;
    Ok(state)
}
/// Reconcile uncertain cancelled work without issuing another provider attempt.
pub async fn reconcile_cancelled(path: &Path, adapter: &dyn Provisioner) -> Result<Execution> {
    let _lock = lock(path)?;
    let mut state = inspect_execution(path)?;
    ensure!(
        state.cancelled,
        "Reconciliation-only cleanup requires a cancelled setup"
    );
    for index in 0..state.steps.len() {
        let step = &state.steps[index];
        if step.state != StepState::Running {
            continue;
        }
        match adapter
            .reconcile(&step.mutation_id, &step.selection)
            .await?
        {
            Reconciliation::Accepted(receipt) => {
                validate_receipt(&step.selection, &receipt)?;
                state.steps[index].state = StepState::Accepted(receipt);
            }
            Reconciliation::NotAccepted => state.steps[index].state = StepState::Cancelled,
            Reconciliation::Unknown => continue,
        }
        write(path, &state)?;
    }
    Ok(state)
}
/// Only accepted owned creates are returned after reconciliation is complete.
pub fn cleanup_owned(state: &Execution) -> Result<Vec<&Receipt>> {
    ensure!(
        !state.steps.iter().any(|s| s.state == StepState::Running),
        "Reconcile unknown outcomes before cleanup"
    );
    Ok(state
        .steps
        .iter()
        .filter_map(|step| match &step.state {
            StepState::Accepted(receipt) if receipt.created => Some(receipt),
            _ => None,
        })
        .collect())
}
