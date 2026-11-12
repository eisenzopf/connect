//! Enforce the coordinated writer protocol at the SQL mutation boundary.
//!
//! The registered function does not authorize an operation. Existing Store
//! methods still check the principal, intent and fresh rows. It requires an
//! actual same-Store Write proof, pinned by this exact SQL unit, for the row
//! footprint supplied by persistent schema triggers. The private phase ledger
//! permit is a narrower exact-transaction exception for gain-only phase metadata.
//! A separate sealed scoped-handoff permit additionally pins real Connection and
//! Record Writers plus complete current Readers for exact native move/retirement
//! row deltas. It never converts a Connection proof into a Conversation Writer.
//! Neither permit acquires fences or reads SQL from inside SQLite.
use crate::{AuthorityGuardSet, AuthorityMode, AuthorityScope, Store};
use connect_protocol::{Error, Result};
use rusqlite::{functions::FunctionFlags, Connection};
use std::sync::{Arc, Mutex, Weak};

pub(crate) mod phase_ledger;
pub(crate) mod schema;

pub(crate) const FUNCTION: &str = "connect_store_writer_guard_v1";
pub(crate) const MESSAGE_BIRTH_FUNCTION: &str = "connect_store_message_birth_v1";

pub(crate) const PARENT_BIRTH_FUNCTION: &str = "connect_store_parent_birth_v1";

#[derive(Clone, Copy, PartialEq, Eq)]
enum BirthKind {
    Message,
    Call,
    InvitedSession,
}
struct BirthKey {
    kind: BirthKind,
    id: String,
    conversation: String,
    nonce: u64,
}

#[derive(Default)]
struct Slot {
    active: bool,
    failed: bool,
    proofs: Vec<Arc<AuthorityGuardSet>>,
    // Identity of the actual primary Connection borrowed by this Unit. This
    // tag is compared only; it is never dereferenced or an authorization grant.
    connection: usize,
    next_birth_nonce: u64,
    birth: Option<BirthKey>,
    phase_ledger: Option<phase_ledger::Active>,
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
            .map_err(|_| Error::storage())?;
        let state = self.clone();
        connection
            .create_scalar_function(
                MESSAGE_BIRTH_FUNCTION,
                2,
                FunctionFlags::SQLITE_UTF8,
                move |context| {
                    let message = context.get::<String>(0)?;
                    let conversation = context.get::<String>(1)?;
                    Ok(i64::from(
                        state.message_birth_matches(&message, &conversation),
                    ))
                },
            )
            .map_err(|_| Error::storage())?;
        let state = self.clone();
        connection
            .create_scalar_function(
                PARENT_BIRTH_FUNCTION,
                3,
                FunctionFlags::SQLITE_UTF8,
                move |context| {
                    let kind = context.get::<String>(0)?;
                    let id = context.get::<String>(1)?;
                    let conversation = context.get::<String>(2)?;
                    let kind = match kind.as_str() {
                        "call" => Some(BirthKind::Call),
                        "invited-session" => Some(BirthKind::InvitedSession),
                        _ => None,
                    };
                    Ok(i64::from(kind.is_some_and(|kind| {
                        state.birth_matches(kind, &id, &conversation)
                    })))
                },
            )
            .map_err(|_| Error::storage())?;
        self.register_phase_ledger(connection)?;
        // Even crate-private SQL can issue COMMIT/ROLLBACK explicitly. Never
        // allow a namespace permit to survive a successful transaction boundary.
        let state = self.clone();
        connection.commit_hook(Some(move || {
            state.slot.lock().map_or(true, |slot| {
                slot.birth.is_some() || slot.phase_ledger.is_some()
            })
        }));
        let state = self.clone();
        connection.rollback_hook(Some(move || {
            let previous = if let Ok(mut slot) = state.slot.lock() {
                slot.birth = None;
                slot.phase_ledger.take()
            } else {
                if let Some(registry) = state.registry.upgrade() {
                    registry.invalidate_all();
                }
                None
            };
            drop(previous);
        }));
        Ok(())
    }

    fn message_birth_matches(&self, message: &str, conversation: &str) -> bool {
        self.birth_matches(BirthKind::Message, message, conversation)
    }
    fn birth_matches(&self, kind: BirthKind, id: &str, conversation: &str) -> bool {
        self.slot.lock().is_ok_and(|slot| {
            slot.active
                && !slot.failed
                && slot.birth.as_ref().is_some_and(|birth| {
                    birth.kind == kind && birth.id == id && birth.conversation == conversation
                })
        })
    }
    pub(crate) fn new_message_birth<'a>(
        self: &Arc<Self>,
        connection: &'a Connection,
        id: &str,
        conversation: &str,
    ) -> Result<NamespaceBirth<'a>> {
        self.new_birth(connection, BirthKind::Message, id, conversation)
    }
    pub(crate) fn new_call_birth<'a>(
        self: &Arc<Self>,
        connection: &'a Connection,
        id: &str,
        conversation: &str,
    ) -> Result<NamespaceBirth<'a>> {
        self.new_birth(connection, BirthKind::Call, id, conversation)
    }
    pub(crate) fn new_invited_session_birth<'a>(
        self: &Arc<Self>,
        connection: &'a Connection,
        id: &str,
        conversation: &str,
    ) -> Result<NamespaceBirth<'a>> {
        self.new_birth(connection, BirthKind::InvitedSession, id, conversation)
    }
    /// Fixed namespace kinds only. The same actual-Unit/write-transaction,
    /// commit/rollback hook and nonce rules apply to every newly born parent.
    fn new_birth<'a>(
        self: &Arc<Self>,
        connection: &'a Connection,
        kind: BirthKind,
        id: &str,
        conversation: &str,
    ) -> Result<NamespaceBirth<'a>> {
        let connection_tag = connection as *const Connection as usize;
        // SQLite does not call commit_hook for a read-only transaction. The
        // actual main database must already own its write transaction, as every
        // production message writer does with BEGIN IMMEDIATE.
        if connection
            .transaction_state(Some(rusqlite::DatabaseName::Main))
            .map_err(|_| Error::storage())?
            != rusqlite::TransactionState::Write
        {
            return Err(Error::storage());
        }
        {
            let slot = self.slot.lock().map_err(|_| Error::storage())?;
            if !slot.active
                || slot.failed
                || slot.connection != connection_tag
                || slot.birth.is_some()
                || slot.phase_ledger.is_some()
                || connection.is_autocommit()
            {
                return Err(Error::storage());
            }
        }
        let exists: bool = connection
            .query_row(
                match kind {
                    BirthKind::Message => "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1)",
                    BirthKind::Call => "SELECT EXISTS(SELECT 1 FROM voice_calls WHERE id=?1)",
                    BirthKind::InvitedSession => {
                        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)"
                    }
                },
                [id],
                |row| row.get(0),
            )
            .map_err(|_| Error::storage())?;
        if exists {
            return Err(Error::new(
                "conflict",
                match kind {
                    BirthKind::Message => "Message identity is unavailable",
                    BirthKind::Call => "Call identity is unavailable",
                    BirthKind::InvitedSession => "Session identity is unavailable",
                },
            ));
        }
        let mut slot = self.slot.lock().map_err(|_| Error::storage())?;
        if !slot.active
            || slot.failed
            || slot.connection != connection_tag
            || slot.birth.is_some()
            || slot.phase_ledger.is_some()
            || connection.is_autocommit()
        {
            return Err(Error::storage());
        }
        let nonce = slot
            .next_birth_nonce
            .checked_add(1)
            .ok_or_else(Error::storage)?;
        slot.next_birth_nonce = nonce;
        slot.birth = Some(BirthKey {
            kind,
            id: id.into(),
            conversation: conversation.into(),
            nonce,
        });
        Ok(NamespaceBirth {
            state: self.clone(),
            nonce,
            _connection: std::marker::PhantomData,
        })
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
        if slot.active
            || slot.failed
            || !connection.is_autocommit()
            || !slot.proofs.is_empty()
            || slot.birth.is_some()
            || slot.phase_ledger.is_some()
        {
            slot.failed = true;
            return Err(Error::storage());
        }
        slot.active = true;
        slot.connection = connection as *const Connection as usize;
        Ok(Unit {
            state: self.clone(),
        })
    }
}

/// A stack-bound permit, never returned by the public Store API. A rollback
/// invalidates its nonce; a stale Drop cannot clear a later transaction's permit.
pub(crate) struct NamespaceBirth<'a> {
    state: Arc<State>,
    nonce: u64,
    _connection: std::marker::PhantomData<&'a Connection>,
}
impl Drop for NamespaceBirth<'_> {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.state.slot.lock() {
            if slot
                .birth
                .as_ref()
                .is_some_and(|birth| birth.nonce == self.nonce)
            {
                slot.birth = None;
            }
        } else if let Some(registry) = self.state.registry.upgrade() {
            registry.invalidate_all();
        }
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
        let (proofs, phase) = if let Ok(mut slot) = self.state.slot.lock() {
            slot.failed |= !clean || slot.birth.is_some() || slot.phase_ledger.is_some();
            slot.birth = None;
            let phase = slot.phase_ledger.take();
            slot.connection = 0;
            slot.active = false;
            (std::mem::take(&mut slot.proofs), phase)
        } else {
            (Vec::new(), None)
        };
        // A WriterNotice may lock its registry on Drop. Do not hold slot here.
        drop(phase);
        drop(proofs);
    }
}

#[cfg(test)]
mod tests;
