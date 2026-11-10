//! Enforce the coordinated writer protocol at the SQL mutation boundary.
//!
//! The registered function does not authorize an operation. Existing Store
//! methods still check the principal, intent and fresh rows. It requires an
//! actual same-Store Write proof, pinned by this exact SQL unit, for the row
//! footprint supplied by persistent schema triggers. It never acquires a fence
//! or reads SQL from inside SQLite.
use crate::{AuthorityGuardSet, AuthorityMode, AuthorityScope, Store};
use connect_protocol::{Error, Result};
use rusqlite::{functions::FunctionFlags, Connection};
use std::sync::{Arc, Mutex, Weak};

pub(crate) mod schema;

pub(crate) const FUNCTION: &str = "connect_store_writer_guard_v1";

#[derive(Default)]
struct Slot {
    active: bool,
    failed: bool,
    proofs: Vec<Arc<AuthorityGuardSet>>,
}

/// One state for the sole primary SQL connection, never packet observers.
pub(crate) struct State {
    slot: Mutex<Slot>,
    registry: Weak<crate::media_authorization::Registry>,
}

impl State {
    pub(crate) fn new(registry: &Arc<crate::media_authorization::Registry>) -> Arc<Self> {
        Arc::new(Self {
            slot: Mutex::new(Slot::default()),
            registry: Arc::downgrade(registry),
        })
    }

    pub(crate) fn require_healthy(&self) -> Result<()> {
        let slot = self.slot.lock().map_err(|_| Error::storage())?;
        if slot.failed {
            Err(Error::storage())
        } else {
            Ok(())
        }
    }

    pub(crate) fn register(self: &Arc<Self>, connection: &Connection) -> Result<()> {
        let state = self.clone();
        connection
            .create_scalar_function(FUNCTION, 2, FunctionFlags::SQLITE_UTF8, move |context| {
                let kind = context.get::<String>(0)?;
                let id = context.get::<Option<String>>(1)?;
                if !state.permits(&kind, id.as_deref()) {
                    return Err(rusqlite::Error::UserFunctionError(Box::new(
                        std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "Coordinated Store mutation required",
                        ),
                    )));
                }
                Ok(1_i64)
            })
            .map_err(|_| Error::storage())
    }

    fn permits(&self, kind: &str, id: Option<&str>) -> bool {
        let Ok(slot) = self.slot.lock() else {
            return false;
        };
        if !slot.active || slot.failed {
            return false;
        }
        // Explicit schema-classified bookkeeping still requires this registered
        // Store SQL unit. It cannot be used for an authority-bearing row.
        if kind == "bookkeeping" {
            return id.is_none();
        }
        let Some(id) = id else {
            return false;
        };
        let scope = match kind {
            "identity" => AuthorityScope::identity(id, AuthorityMode::Write),
            "conversation" => AuthorityScope::conversation(id, AuthorityMode::Write),
            "record" => AuthorityScope::record(id, AuthorityMode::Write),
            _ => return false,
        };
        scope.is_ok_and(|scope| {
            slot.proofs
                .iter()
                .any(|proof| proof.covers(&[scope.clone()]))
        })
    }

    /// Called only after the primary SQL mutex is owned. No permit survives a
    /// connection unit or can be inherited by its next caller.
    pub(crate) fn enter(self: &Arc<Self>, connection: &Connection) -> Result<Unit> {
        let mut slot = self.slot.lock().map_err(|_| Error::storage())?;
        if slot.active || slot.failed || !connection.is_autocommit() || !slot.proofs.is_empty() {
            slot.failed = true;
            return Err(Error::storage());
        }
        slot.active = true;
        Ok(Unit {
            state: self.clone(),
        })
    }
}

pub(crate) struct Unit {
    state: Arc<State>,
}

impl Unit {
    pub(crate) fn bind(
        &self,
        store: &Store,
        proof: &Arc<AuthorityGuardSet>,
        required: &[AuthorityScope],
    ) -> Result<()> {
        // `required` is freshly derived by the same mutator before DML. A
        // reader, foreign Store or insufficient writer cannot be converted.
        if !Arc::ptr_eq(&self.state, &store.authority_writer_boundary)
            || required.is_empty()
            || required.iter().any(|scope| scope.write_scope() != *scope)
            || !store.authority_covers(proof, required)
        {
            return Err(Error::forbidden());
        }
        let mut slot = self.state.slot.lock().map_err(|_| Error::storage())?;
        if !slot.active || slot.failed {
            return Err(Error::storage());
        }
        if !slot.proofs.iter().any(|old| Arc::ptr_eq(old, proof)) {
            if slot.proofs.len() >= 32 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "SQL mutation proof capacity exceeded",
                ));
            }
            slot.proofs.push(proof.clone());
        }
        Ok(())
    }

    /// The SQL mutex is still held. Complete or roll back SQL before dropping
    /// pinned Write guards and their publication-exclusion notices.
    pub(crate) fn finish(&self, connection: &Connection) {
        let clean = connection.is_autocommit()
            || (connection.execute_batch("ROLLBACK").is_ok() && connection.is_autocommit());
        if !clean {
            if let Some(registry) = self.state.registry.upgrade() {
                registry.invalidate_all();
            }
        }
        let proofs = if let Ok(mut slot) = self.state.slot.lock() {
            slot.failed |= !clean;
            slot.active = false;
            std::mem::take(&mut slot.proofs)
        } else {
            Vec::new()
        };
        // A WriterNotice may lock its registry on Drop. Do not hold slot here.
        drop(proofs);
    }
}

#[cfg(test)]
mod tests;
