//! A separate integration executable gives this race a fresh process default.
#[test]
fn concurrent_provider_clients_initialize_without_poisoning_vapi() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let threads = (0..16)
        .map(|i| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..8 {
                    if i % 2 == 0 {
                        assert!(
                            connect_connectors::Vapi::new("fixture", "http://127.0.0.1").is_ok()
                        );
                    } else {
                        assert!(
                            connect_connectors::Telnyx::new("fixture", "http://127.0.0.1").is_ok()
                        );
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        assert!(thread.join().is_ok());
    }
}
