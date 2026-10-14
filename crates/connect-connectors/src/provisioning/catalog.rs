//! Catalogs contain resource metadata only, never keys or inferred enrollment.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Assistant,
    Number,
    Trunk,
    MessagingProfile,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub kind: ResourceKind,
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
}
/// A complete catalog or explicit truncation. Partial lists cannot authorize creates.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// Every explicitly requested provider authenticated; omitted providers are unknown.
    pub authenticated: bool,
    /// Complete across both providers and bounded pages; partial discovery cannot authorize creates.
    pub complete: bool,
    pub resources: Vec<Resource>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selection {
    /// Exact ID reuse never changes the resource, including unrelated account assets.
    Reuse {
        kind: ResourceKind,
        id: String,
        required_capabilities: Vec<String>,
    },
    /// Explicitly selected create, not an implicit reaction to empty discovery.
    Create { kind: ResourceKind, name: String },
}
/// Approved intent remains immutable through execution and restart.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub profile: String,
    pub selections: Vec<Selection>,
}
/// Read-only preview of exact actions and remaining prerequisite gaps.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preview {
    pub profile: String,
    pub selections: Vec<Selection>,
    pub gaps: Vec<String>,
    pub purchases: bool,
    pub public_ingress_created: bool,
}
impl Intent {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.profile == "connect-provider-setup/1"
                && !self.selections.is_empty()
                && self.selections.len() <= 16,
            "Unsupported or empty setup intent"
        );
        for selection in &self.selections {
            ensure!(
                !matches!(
                    selection,
                    Selection::Create {
                        kind: ResourceKind::Number,
                        ..
                    }
                ),
                "Number purchasing is a separate explicitly authorized operation"
            );
            let (value, capabilities) = match selection {
                Selection::Reuse {
                    id,
                    required_capabilities,
                    ..
                } => (id, Some(required_capabilities)),
                Selection::Create { name, .. } => (name, None),
            };
            ensure!(
                !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
                "Invalid resource selector"
            );
            if let Some(caps) = capabilities {
                ensure!(
                    caps.len() <= 16
                        && caps.iter().all(|c| !c.is_empty()
                            && c.len() <= 64
                            && c.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))),
                    "Invalid capability selector"
                );
            }
        }
        Ok(())
    }
}
/// Preview performs no mutation. Authentication or ambiguity errors are redacted.
pub fn preview(catalog: &Catalog, intent: &Intent) -> Result<Preview> {
    intent.validate()?;
    ensure!(
        catalog.authenticated,
        "Provider authentication rejected; replace the selected server secret reference"
    );
    ensure!(
        catalog.resources.len() <= 10000,
        "Catalog exceeds resource bound"
    );
    let mut seen = HashSet::new();
    for resource in &catalog.resources {
        ensure!(
            seen.insert((&resource.kind, &resource.id)),
            "Ambiguous resource IDs; reconcile catalog before selection"
        );
    }
    let mut gaps = Vec::new();
    for selection in &intent.selections {
        match selection {
            Selection::Reuse {
                kind,
                id,
                required_capabilities,
            } => {
                let resource = catalog
                    .resources
                    .iter()
                    .find(|r| &r.kind == kind && &r.id == id)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Selected resource is absent or inaccessible; refresh catalog"
                        )
                    })?;
                for capability in required_capabilities {
                    if !resource.capabilities.contains(capability) {
                        gaps.push(format!("Selected {:?} lacks {capability}", kind));
                    }
                }
            }
            Selection::Create { .. } => ensure!(
                catalog.complete,
                "Catalog is truncated; finish discovery before approving creates"
            ),
        }
    }
    if intent.selections.iter().any(|s| {
        matches!(
            s,
            Selection::Reuse {
                kind: ResourceKind::Number,
                ..
            } | Selection::Create {
                kind: ResourceKind::MessagingProfile,
                ..
            }
        )
    }) {
        gaps.push("SMS needs separate reviewed enrollment and fresh eligibility; selecting a number grants no consent".into());
        gaps.push(
            "Inbound SMS needs an independently configured reachable signed-callback endpoint"
                .into(),
        );
    }
    if intent.selections.iter().any(|s| {
        matches!(
            s,
            Selection::Reuse {
                kind: ResourceKind::Trunk,
                ..
            } | Selection::Create {
                kind: ResourceKind::Trunk,
                ..
            }
        )
    }) {
        gaps.push("SIP registration and inbound/outbound RTP need separate observations; HTTPS cannot verify them".into());
    }
    Ok(Preview {
        profile: intent.profile.clone(),
        selections: intent.selections.clone(),
        gaps,
        purchases: false,
        public_ingress_created: false,
    })
}
