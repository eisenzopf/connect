//! Named trusted administration; Store owns the actual transaction fences.
use crate::Service;
use connect_protocol::{incoming_contacts::*, Error, Result};
async fn database<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| Error::storage())?
}
impl Service {
    pub async fn execute_incoming_contact(
        &self,
        credential: String,
        command: IncomingContactCommand,
    ) -> Result<IncomingContactOutcome> {
        command.validate()?;
        let mutation = matches!(
            command,
            IncomingContactCommand::Resolve(_) | IncomingContactCommand::Reject { .. }
        );
        let store = self.store.clone();
        let result = match command {
            IncomingContactCommand::List { after, limit } => {
                database(move || store.incoming_contacts(&credential, after.as_deref(), limit))
                    .await
                    .map(|page| IncomingContactOutcome::Page { page })
            }
            IncomingContactCommand::Get { contact_id } => {
                database(move || store.incoming_contact(&credential, &contact_id))
                    .await
                    .map(|contact| IncomingContactOutcome::Contact { contact })
            }
            IncomingContactCommand::Reject {
                operation_id,
                contact_id,
                expected_revision,
                reason,
            } => database(move || {
                store.reject_incoming_contact(
                    &credential,
                    &operation_id,
                    &contact_id,
                    expected_revision,
                    &reason,
                )
            })
            .await
            .map(|contact| IncomingContactOutcome::Contact { contact }),
            IncomingContactCommand::Resolve(selection) => {
                let key = credential.clone();
                let id = selection.contact_id.clone();
                let current = database(move || store.incoming_contact(&key, &id)).await?;
                let contact = match current.kind {
                    IncomingContactKind::Sms => {
                        let store = self.store.clone();
                        database(move || {
                            store.resolve_incoming_sms_contact(
                                &credential,
                                &selection.operation_id,
                                &selection.contact_id,
                                selection.expected_revision,
                                &selection.candidate_id,
                            )
                        })
                        .await
                    }
                    IncomingContactKind::Voice => {
                        self.resolve_incoming_voice_contact(credential, selection)
                            .await
                    }
                };
                contact.map(|contact| IncomingContactOutcome::Contact { contact })
            }
        };
        if mutation && result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
