//! A weak link to one bounded original publication, never an idle wire grant.
use super::*;
use crate::canonical_media::data::DataInvocation;
use rvoip_uctp::v2::auth_dispatch::AuthorizedEnvelope;

fn binding_contention<T>(error: std::sync::TryLockError<T>) -> connect_protocol::Error {
    match error {
        std::sync::TryLockError::WouldBlock => {
            connect_protocol::Error::new("authority-busy", "Native data invocation binding is busy")
        }
        std::sync::TryLockError::Poisoned(_) => connect_protocol::Error::forbidden(),
    }
}

pub(crate) fn bind(
    a: &AuthorizedEnvelope,
    invocation: &Arc<DataInvocation>,
) -> connect_protocol::Result<()> {
    let h = a
        .authority::<StoreInvocationAuthority>()
        .ok_or_else(connect_protocol::Error::forbidden)?;
    let g = h
        .guard::<Arc<Grant>>()
        .ok_or_else(connect_protocol::Error::forbidden)?;
    let PeerCommand::NativeStream(wire) = &g.request.command else {
        return Err(connect_protocol::Error::forbidden());
    };
    if wire.command != connect_protocol::native_streams::Command::Data
        || !invocation.matches(a)
        || !g.proof.current()
        || a.envelope().id() != g.invocation_id
        || crate::canonical_native_streams::intent(a.envelope()).as_ref() != Ok(wire)
    {
        return Err(connect_protocol::Error::forbidden());
    }
    let mut saved = g.native_data.try_lock().map_err(binding_contention)?;
    if saved
        .upgrade()
        .is_some_and(|old| !Arc::ptr_eq(&old, invocation))
    {
        return Err(connect_protocol::Error::forbidden());
    }
    *saved = Arc::downgrade(invocation);
    Ok(())
}
pub(crate) fn invocation(a: &AuthorizedEnvelope) -> Option<Arc<DataInvocation>> {
    let g = a
        .authority::<StoreInvocationAuthority>()?
        .guard::<Arc<Grant>>()?;
    let i = g.native_data.try_lock().ok()?.upgrade()?;
    i.matches(a).then_some(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_native_data_binding_is_retryable_without_installing_an_invocation() {
        let slot = Mutex::<Weak<DataInvocation>>::new(Weak::new());
        let held = slot.lock().unwrap();
        assert_eq!(
            slot.try_lock()
                .map_err(binding_contention)
                .err()
                .unwrap()
                .code,
            "authority-busy"
        );
        assert!(held.upgrade().is_none());
        drop(held);
        assert!(slot.try_lock().unwrap().upgrade().is_none());
    }

    #[test]
    fn poisoned_native_data_binding_remains_a_definitive_refusal() {
        let slot = Arc::new(Mutex::<Weak<DataInvocation>>::new(Weak::new()));
        let owner = slot.clone();
        assert!(std::thread::spawn(move || {
            let _held = owner.lock().unwrap();
            panic!("controlled native data binding poison");
        })
        .join()
        .is_err());
        assert_eq!(
            slot.try_lock()
                .map_err(binding_contention)
                .err()
                .unwrap()
                .code,
            "forbidden"
        );
    }
}
