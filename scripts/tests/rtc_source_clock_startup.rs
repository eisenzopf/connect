#[cfg(test)]
mod tests {
    use rtc_interceptor::{
        Interceptor, NoopInterceptor, Packet, Registry, SenderClockAnchor, SenderClockHandle,
        SenderClockUpdate, SenderReportBuilder, SenderReportInterceptor, StreamInfo, TaggedPacket,
    };
    use sansio::Protocol;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    fn chain(clock: SenderClockHandle) -> SenderReportInterceptor<NoopInterceptor> {
        Registry::new()
            .with(SenderReportBuilder::new().with_clock_handle(clock).build())
            .build()
    }
    fn info() -> StreamInfo {
        StreamInfo {
            ssrc: 99,
            clock_rate: 48000,
            ..Default::default()
        }
    }
    fn update(alive: Arc<AtomicBool>, ready: bool) -> SenderClockUpdate {
        let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let ntp = ((time.as_secs() + 2_208_988_800) << 32)
            | ((u64::from(time.subsec_nanos()) << 32) / 1_000_000_000);
        SenderClockUpdate {
            source_id: 1,
            generation: 1,
            rtp_offset: 7,
            supported: true,
            anchor: ready.then_some(SenderClockAnchor {
                ntp_time: ntp,
                rtp_time: 1000,
                clock_rate: 48000,
            }),
            is_current: Arc::new(move || alive.load(Ordering::Acquire)),
        }
    }
    fn send(chain: &mut SenderReportInterceptor<NoopInterceptor>, now: Instant, sequence: u16) {
        chain
            .handle_write(TaggedPacket {
                now,
                transport: Default::default(),
                message: Packet::Rtp(rtp::Packet {
                    header: rtp::Header {
                        ssrc: 99,
                        sequence_number: sequence,
                        timestamp: u32::from(sequence) * 960,
                        ..Default::default()
                    },
                    payload: vec![1, 2, 3].into(),
                    ..Default::default()
                }),
            })
            .unwrap();
        assert!(matches!(
            chain.poll_write().unwrap().message,
            Packet::Rtp(_)
        ));
    }
    #[test]
    fn first_current_source_report_is_prompt_and_once_without_resetting_periodic_cadence() {
        let clock = SenderClockHandle::default();
        let mut c = chain(clock.clone());
        c.bind_local_stream(&info());
        let start = Instant::now();
        c.handle_timeout(start).unwrap();
        assert!(c.poll_write().is_none());
        let alive = Arc::new(AtomicBool::new(true));
        send(&mut c, start, 1);
        assert!(clock.apply_after_rtp(99, Some(update(alive.clone(), false))));
        c.handle_timeout(start + Duration::from_millis(100))
            .unwrap();
        assert!(c.poll_write().is_none());
        let ready = start + Duration::from_millis(250);
        send(&mut c, ready, 2);
        let timing = update(alive.clone(), true);
        let anchor = timing.anchor.unwrap();
        assert!(clock.apply_after_rtp(99, Some(timing.clone())));
        assert!(
            c.poll_timeout().unwrap() <= ready,
            "first source mapping waits for the periodic timer"
        );
        c.handle_timeout(ready).unwrap();
        let Packet::Rtcp(packets) = c
            .poll_write()
            .expect("first current source SR was not emitted promptly")
            .message
        else {
            panic!("expected RTCP")
        };
        let sr = packets[0]
            .as_any()
            .downcast_ref::<rtcp::sender_report::SenderReport>()
            .unwrap();
        assert_eq!(sr.rtp_time, anchor.rtp_at(sr.ntp_time).wrapping_add(7));
        assert_eq!((sr.packet_count, sr.octet_count), (2, 6));
        assert!(c.poll_write().is_none());
        send(&mut c, ready + Duration::from_millis(20), 3);
        assert!(clock.apply_after_rtp(99, Some(timing)));
        c.bind_local_stream(&info()); // Harmless re-description cannot schedule another urgent report.
        c.handle_timeout(ready + Duration::from_millis(20)).unwrap();
        assert!(c.poll_write().is_none());
        assert_eq!(c.poll_timeout().unwrap(), start + Duration::from_secs(1));
        c.handle_timeout(start + Duration::from_secs(1)).unwrap();
        assert!(matches!(c.poll_write().unwrap().message, Packet::Rtcp(_)));
        assert!(c.poll_write().is_none());
    }
    #[test]
    fn source_metadata_without_media_pending_and_retired_clocks_never_emit_reports() {
        let clock = SenderClockHandle::default();
        let mut c = chain(clock.clone());
        c.bind_local_stream(&info());
        let start = Instant::now();
        let alive = Arc::new(AtomicBool::new(true));
        assert!(clock.apply_after_rtp(99, Some(update(alive.clone(), true))));
        c.handle_timeout(start).unwrap();
        assert!(c.poll_write().is_none());
        send(&mut c, start, 1);
        assert!(clock.apply_after_rtp(99, Some(update(alive.clone(), false))));
        // Use a fresh binding to test genuinely pending metadata, rather than the
        // same-generation rule that intentionally keeps a newer known anchor.
        c.unbind_local_stream(&info());
        c.bind_local_stream(&info());
        send(&mut c, start, 2);
        assert!(clock.apply_after_rtp(99, Some(update(alive.clone(), false))));
        c.handle_timeout(start + Duration::from_secs(2)).unwrap();
        assert!(c.poll_write().is_none());
        alive.store(false, Ordering::Release);
        assert!(!clock.apply_after_rtp(99, Some(update(alive, true))));
        c.handle_timeout(start + Duration::from_secs(3)).unwrap();
        assert!(c.poll_write().is_none());
    }
    #[test]
    fn untouched_native_sender_keeps_its_original_periodic_cadence() {
        let clock = SenderClockHandle::default();
        let mut c = chain(clock);
        c.bind_local_stream(&info());
        let start = Instant::now();
        c.handle_timeout(start).unwrap();
        send(&mut c, start, 1);
        c.handle_timeout(start + Duration::from_millis(250))
            .unwrap();
        assert!(c.poll_write().is_none());
        assert_eq!(c.poll_timeout().unwrap(), start + Duration::from_secs(1));
        c.handle_timeout(start + Duration::from_secs(1)).unwrap();
        assert!(matches!(c.poll_write().unwrap().message, Packet::Rtcp(_)));
    }
}
