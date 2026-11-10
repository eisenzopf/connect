//! Failure-only redacted observations, never a retry or an acceptance rule.
use super::*;
use rusqlite::{Connection, OpenFlags, Params};

fn query(db: &Connection, sql: &str, params: impl Params) -> Value {
    let result =
        (|| -> rusqlite::Result<Vec<Value>> {
            let mut statement = db.prepare(sql)?;
            let rows = statement.query_map(params, |row| row.get::<_, String>(0))?;
            rows.take(16)
                .map(|row| {
                    Ok(serde_json::from_str(&row?)
                        .unwrap_or(serde_json::json!({"unavailable":"json"})))
                })
                .collect()
        })();
    match result {
        Ok(rows) => serde_json::json!({"rows":rows,"row_cap":16}),
        Err(error) => {
            serde_json::json!({"unavailable":"sqlite","sqlite_code":error.sqlite_error().map(|e|e.extended_code)})
        }
    }
}

pub(super) fn original_root_state(
    f: &Fixture,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
) -> Value {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    // This independent connection cannot acquire Store/domain guards,
    // recover work or mutate the original operation. Read actual WAL
    // state; never use immutable mode to ignore committed WAL rows.
    let history = (|| -> rusqlite::Result<Value> {
        let db = Connection::open_with_flags(
            &f.config.database,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::ZERO)?;
        db.pragma_update(None, "query_only", true)?;
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        let mut ticks = 0usize;
        db.progress_handler(
            1000,
            Some(move || {
                ticks += 1;
                ticks >= 100 || std::time::Instant::now() >= deadline
            }),
        );
        let record = root
            .record_id
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("");
        let original: Option<String> = db.query_row(
        "SELECT json_object('state',state,'revision',revision,'cancel_requested',cancel_requested,'lease_expires_at',lease_expires_at,'expires_at',expires_at) FROM operations WHERE record_id=?1 AND operation_id=?2 LIMIT 1",
        rusqlite::params![record,command.operation_id], |row|row.get(0),
    ).optional()?;
        let mut query = db.prepare(
        "SELECT json_object('provider',substr(e.provider,1,64),'state',e.state,'attempt',e.attempt,'submission_generation',e.submission_generation) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.record_id=?1 AND o.operation_id=?2 ORDER BY e.id LIMIT 8",
    )?;
        let rows = query.query_map(rusqlite::params![record, command.operation_id], |row| {
            row.get::<_, String>(0)
        })?;
        let effects = rows
            .map(|row| row.map(|text| serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(
            json!({"original_operation":original.and_then(|text|serde_json::from_str::<Value>(&text).ok()),"effects":effects}),
        )
    })();
    let history = match history {
        Ok(value) => value,
        Err(error) => {
            json!({"unavailable":"readonly_sqlite","sqlite_code":error.sqlite_error().map(|value|value.extended_code)})
        }
    };
    history
}

// Existing aggregate counters only. Sampling creates no packet reader, probe,
// task or domain authority. Counter identities are explicit; these are not a
// per-route causal proof when other media is active in the same fixture.
fn client_stats(client: &sdk::Client) -> Value {
    match client.media_stats() {
        Ok(s) => json!({"received_datagrams":s.received_datagrams,"rtp_packets":s.rtp_packets,
            "clock_reports":s.clock_reports,"malformed_datagrams":s.malformed_datagrams,
            "unknown_handles":s.unknown_handles,"overflow_drops":s.overflow_drops,
            "control_drops":s.control_drops,"binding_change_drops":s.binding_change_drops,
            "queued_packets":s.queued_packets,"queued_bytes":s.queued_bytes,
            "feedback_datagrams":s.feedback_datagrams,"feedback_drops":s.feedback_drops}),
        Err(_) => json!({"unavailable":"client-media-stats"}),
    }
}
pub(super) fn safe_media_sample(
    f: &Fixture,
    original_source: &sdk::Client,
    retained: &sdk::Client,
    connection: &sdk::Connection,
    stream: &sdk::Stream,
    target: &NativeVideoDatagrams,
) -> Value {
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let runtime = f.server.service.canonical_runtime_snapshot().map(|s| json!({
            "media_readers":s.media_readers,"media_datagrams_accepted":s.media_datagrams_accepted,
            "media_datagrams_rejected":s.media_datagrams_rejected}));
        let core = f.server.service.canonical_runtime_media_diagnostics().map(|d| {
            let s = d.snapshot();
            let queues = s.queues.into_iter().filter(|(stage,_)| stage.kind()=="video").take(8)
                .map(|(stage,q)| json!({"stage":stage.name(),"capacity":q.capacity_packets,
                    "observations":q.observations,"high_water":q.sampled_high_water_packets,
                    "enqueue_full":q.enqueue_full_observations,"stale_discards":q.stale_forwarding_discards,
                    "max_forwarded_age_ms":q.max_forwarded_frame_age_ms})).collect::<Vec<_>>();
            let forwarding = s.forwarding.into_iter().take(3).map(|(stage,v)| json!({
                "stage":stage.name(),"samples":v.samples,"max_us":v.max_us,
                "missing_ingress":v.missing_ingress_observations,"handoff_timeouts":v.handoff_timeouts})).collect::<Vec<_>>();
            let quic = s.quic.map(|v| json!({"samples":v.samples,"accepted_datagrams":v.accepted_datagrams,
                "too_large":v.too_large,"disabled":v.disabled,"connection_lost":v.connection_lost,
                "unsupported_by_peer":v.unsupported_by_peer,"video_budget_observations":v.video_budget_observations,
                "video_packetizer_discards":v.video_packetizer_discards,"video_no_datagram_budget":v.video_no_datagram_budget}));
            json!({"queues":queues,"forwarding":forwarding,"quic":quic,
                "scope":"installed-Core aggregates; absent means unobserved, not zero"})
        });
        let allocation = match connection.native_negotiation() {
            Ok(Some(n)) => {
                let binding=n.streams.iter().find(|b| &b.id==stream.id());
                let feedback=n.video_feedback.get(stream.id());
                json!({"connection_matches":connection.id()==stream.connection_id(),
                    "stream_revision":n.stream_revision,"binding_present":binding.is_some(),
                    "stream_local_id":binding.and_then(|b|b.stream_local_id),
                    "payload_type":binding.and_then(|b|b.codec["params"]["payload_type"].as_u64()),
                    "direction":binding.map(|b|format!("{:?}",b.direction)),
                    "feedback_present":feedback.is_some(),
                    "stream_generation":feedback.map(|b|b.stream_generation),
                    "connection_generation":feedback.map(|b|b.connection_generation)})
            }
            Ok(None)=>json!({"unavailable":"no-local-negotiation"}),
            Err(_)=>json!({"unavailable":"local-negotiation"}),
        };
        #[cfg(feature = "test-support")]
        let target_notices = target.verified_video_receiver_notice_count_for_test().ok();
        #[cfg(not(feature = "test-support"))]
        let target_notices = { let _ = target; None::<usize> };
        json!({"original_source_sdk":client_stats(original_source),"retained_receiver_sdk":client_stats(retained),
            "replacement_target_sdk":{"unavailable":"raw CanonicalClient has no Client.media_stats"},
            "replacement_target_verified_receiver_notices":target_notices,
            "retained_allocation":allocation,"runtime":runtime,"core":core})
    })).unwrap_or_else(|_|json!({"unavailable":"diagnostic-panicked"}))
}

pub(super) fn public_uplink(
    f: &Fixture,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    target: &str,
    retained: &sdk::Stream,
    before: &Value,
    failed: &Value,
    camera_handle: u16,
    first_sequence: u16,
    next_sequence: u16,
    previous_epoch: &sdk::VideoReceiveEpoch,
) {
    let mut report = serde_json::json!({
        "stage":"first-public-target-to-retained-uplink",
        "expected_rtp_timestamp":3000,
        "private_decode_completed":true,"root_completed_observed":true,
        "concurrent_snapshot":true,"payloads_retained":false,
        "packet_observer":f.server.store.packet_observer_claim_diagnostics(),
        "media_before_send":before,"media_at_failure":failed,
        "original_uplink":{"target_camera_handle":camera_handle,"payload_type":96,"ssrc":0x74657374u32,
            "rtp_timestamp":3000,"first_sequence":first_sequence,"next_sequence":next_sequence,
            "locally_submitted_packets":next_sequence.wrapping_sub(first_sequence)},
        "previous_retained_receiver_history":{"epoch_sha256":format!("{:x}",Sha256::digest(previous_epoch.epoch.as_bytes())),
            "incarnation_sha256":format!("{:x}",Sha256::digest(previous_epoch.incarnation.as_bytes())),
            "revision":previous_epoch.revision,"ssrc":previous_epoch.ssrc,
            "first_sequence":previous_epoch.first_sequence,"first_timestamp":previous_epoch.first_timestamp},
        "limits":{"rows_per_query":16,"sql_vm_ops":100000,"sql_vm_cooperative_wall_ms":100,"busy_wait_ms":0,"output_bytes":16384},
        "limitations":"State after failure; no proof of exact lost-packet stage or receiver-task liveness. SQL statements may observe different commits."
    });
    let observation = (|| -> rusqlite::Result<Value> {
        let db = Connection::open_with_flags(
            &f.config.database,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::ZERO)?;
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        let mut calls = 0usize;
        db.progress_handler(
            1000,
            Some(move || {
                calls += 1;
                calls >= 100 || std::time::Instant::now() >= deadline
            }),
        );
        let record = root
            .record_id
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("");
        let source = command
            .connection_id
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("");
        let original=query(&db,
            "SELECT json_object('state',state,'revision',revision,'cancel_requested',cancel_requested,'lease_expires_at',lease_expires_at,'expires_at',expires_at) FROM operations WHERE record_id=?1 LIMIT 1",[record]);
        let steps=query(&db,
            "SELECT json_object('phase',s.phase,'started',s.started,'observed',s.observation_json IS NOT NULL,'effect_state',e.state) FROM (SELECT phase,started,observation_json,effect_id FROM native_replacement_steps WHERE operation_record_id=?1 UNION ALL SELECT phase,started,observation_json,effect_id FROM native_replacement_media_steps WHERE operation_record_id=?1) s LEFT JOIN operation_effects e ON e.id=s.effect_id ORDER BY s.phase LIMIT 16",[record]);
        let mut connections = serde_json::Map::new();
        for (role, id) in [
            ("source", source),
            ("target", target),
            ("retained", retained.connection_id().as_str()),
        ] {
            connections.insert(role.into(),query(&db,
                "SELECT json_object('state',c.state,'generation',c.generation,'revision',c.revision,'stream_revision',json_extract(c.readiness_json,'$.stream_revision'),'has_readiness',c.readiness_json IS NOT NULL,'has_runtime',c.observed_runtime_id IS NOT NULL,'lifetime_present',l.connection_id IS NOT NULL,'lifetime_retired',l.retired,'lifetime_generation',l.generation,'runtime_matches_lifetime',c.observed_runtime_id=l.runtime_epoch) FROM connections c LEFT JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE c.id=?1 LIMIT 1",[id]));
        }
        let selectors=query(&db,
            "SELECT json_object('role',CASE WHEN o.connection_id=?1 THEN 'target' ELSE 'retained' END,'command',o.command_type,'state',o.state,'revision',o.revision,'cancel_requested',o.cancel_requested,'closed',n.closed,'effect_started',n.effect_started,'effect_state',e.state,'generation',n.generation) FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id LEFT JOIN operation_effects e ON e.id=n.effect_id WHERE o.conversation_id=?3 AND o.connection_id IN (?1,?2) AND o.command_type IN ('stream.subscribe','stream.unsubscribe','stream.reconcile') ORDER BY o.record_id LIMIT 16",rusqlite::params![target,retained.connection_id().as_str(),command.conversation_id.as_str()]);
        Ok(
            serde_json::json!({"original_operation":original,"replacement_steps":steps,"connections":connections,"selectors":selectors}),
        )
    })();
    report["control_state"] = match observation {
        Ok(value) => value,
        Err(error) => {
            serde_json::json!({"unavailable":"sqlite_open_or_configuration","sqlite_code":error.sqlite_error().map(|e|e.extended_code)})
        }
    };
    let encoded = report.to_string();
    if encoded.len() <= 16384 {
        eprintln!("replacement public uplink failure diagnostic: {encoded}");
    } else {
        eprintln!("replacement public uplink failure diagnostic: {{\"unavailable\":\"output-cap\",\"bytes\":{}}}",encoded.len());
    }
}
