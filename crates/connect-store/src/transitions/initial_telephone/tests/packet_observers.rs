//! Actual initial SIP media projection: command SQL independence and fail closed
//! bounded observer custody. No synthetic packet grant or copied read algorithm.
use super::*;

fn answered(f: &Fixture) -> Arc<InitialTelephoneMediaOrigin> {
    let signal = Arc::new(f.signal());
    let capture = signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap();
    f.store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .unwrap()
}

#[test]
fn current_initial_sip_read_uses_committed_wal_while_command_sql_is_owned() {
    let f = Fixture::new();
    let media = answered(&f);
    let command = f.store.connection().unwrap();
    let read = f
        .store
        .try_initial_telephone_media_read(media.clone())
        .unwrap();
    assert!(read.current());
    assert!(!read.is_connected(), "an answer does not invent readiness");
    assert_eq!(read.origin().connection_id(), media.connection_id());
    assert_eq!(read.endpoint_actor().identity_id, f.ids[1]);
    assert_eq!(read.member().participant_id, f.parts[1]);
    // A returned media lease retains the file interval, never observer SQL.
    let observers: Vec<_> = f
        .store
        .packet_connections
        .iter()
        .map(|c| c.try_lock().unwrap())
        .collect();
    assert_eq!(observers.len(), crate::packet_observation::OBSERVERS);
    assert!(observers[0]
        .execute("CREATE TABLE forbidden_packet_write(value)", [])
        .is_err());
    drop(observers);
    drop(read);
    drop(command);
}

#[test]
fn exhausted_observers_refuse_then_recover_without_preserving_withdrawn_rights() {
    let f = Fixture::new();
    let media = answered(&f);
    let observers: Vec<_> = f
        .store
        .packet_connections
        .iter()
        .map(|c| c.try_lock().unwrap())
        .collect();
    assert_eq!(
        f.store
            .try_initial_telephone_media_read(media.clone())
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(observers);
    drop(
        f.store
            .try_initial_telephone_media_read(media.clone())
            .unwrap(),
    );
    f.store
        .revoke(&f.store.revalidate(&f.keys[0]).unwrap(), &f.keys[1])
        .unwrap();
    let command = f.store.connection().unwrap();
    assert_eq!(
        f.store
            .try_initial_telephone_media_read(media)
            .unwrap_err()
            .code,
        Error::unauthorized().code
    );
    drop(command);
}
