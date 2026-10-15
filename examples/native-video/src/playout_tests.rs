#![allow(clippy::unwrap_used)]
use super::*;
#[test]
fn reorders_before_release_across_wrap_and_rejects_late_replay() {
    let start = Instant::now();
    let mut track = Track::new(90000, 8);
    track.push(u32::MAX - 1000, 1, start);
    track.push(5000, 3, start + Duration::from_millis(20));
    track.push(2000, 2, start + Duration::from_millis(40));
    assert!(track.due(start + Duration::from_millis(119)).is_none());
    assert_eq!(track.pop().frame, 1);
    assert_eq!(track.pop().frame, 2);
    assert_eq!(track.pop().frame, 3);
    track.push(2000, 2, start + Duration::from_millis(180));
    assert!(track.queue.is_empty());
}
#[test]
fn queues_are_bounded_and_stale_frames_cannot_be_released() {
    let start = Instant::now();
    let mut track = Track::new(8000, 10);
    for frame in 0..100 {
        track.push(frame * 160, frame, start);
    }
    assert_eq!(track.peak, 10);
    assert_eq!(track.queue.len(), 10);
    track.expire(start + STALE);
    assert!(track.queue.is_empty());
    assert_eq!(track.dropped, 100);
}
#[test]
fn future_sender_clock_cannot_extend_the_local_release_delay() {
    let start = Instant::now();
    let mut track = Track::new(90000, 8);
    track.push(0, 1, start);
    track.push(90000, 2, start + Duration::from_millis(33));
    assert_eq!(track.pop().frame, 1);
    assert_eq!(
        track.next(),
        Some(start + Duration::from_millis(33) + DELAY)
    );
    let now = start + Duration::from_millis(33) + DELAY;
    track.expire(now);
    assert!(track.due(now).is_some());
    assert_eq!(track.pop().frame, 2);
    assert_eq!(track.dropped, 0);
}
#[test]
fn reordered_due_times_wait_for_the_head_or_an_actual_expiry() {
    let start = Instant::now();
    let mut track = Track::new(1000, 8);
    track.push(0, 0, start);
    track.push(200, 2, start + Duration::from_millis(10));
    track.push(100, 1, start + Duration::from_millis(100));
    assert_eq!(track.pop().frame, 0);
    // Frame 2 is locally due at 130ms, but frame 1 cannot release until
    // 220ms. Its expiry at 210ms is the first event that can change state.
    let now = start + Duration::from_millis(130);
    assert!(track.due(now).is_none());
    assert_eq!(track.next(), Some(start + Duration::from_millis(210)));
    track.expire(start + Duration::from_millis(210));
    assert_eq!(track.dropped, 1);
    assert_eq!(track.next(), Some(start + Duration::from_millis(220)));
    assert!(track.due(start + Duration::from_millis(220)).is_some());
    assert_eq!(track.pop().frame, 1);
    assert_eq!(track.next(), None);
}
#[test]
fn audio_and_video_use_same_release_delay_with_distinct_rtp_origins() {
    let start = Instant::now();
    let mut audio = Track::new(8000, 10);
    let mut video = Track::new(90000, 8);
    audio.push(100000, 1, start);
    video.push(999999999, 1, start);
    assert_eq!(audio.next(), video.next());
    assert_eq!(audio.next(), Some(start + DELAY));
}

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::time::{advance, Sleep};

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    flush_delay: Option<Duration>,
    flushing: Option<Pin<Box<Sleep>>>,
    partial_bound: Option<usize>,
    failure: bool,
    blocking_flush: Option<Duration>,
}
impl AsyncWrite for Sink {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.failure {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let available = self
            .partial_bound
            .map_or(bytes.len(), |bound| bound.saturating_sub(self.bytes.len()));
        if available == 0 {
            return Poll::Pending;
        }
        let n = bytes.len().min(available);
        self.bytes.extend_from_slice(&bytes[..n]);
        Poll::Ready(Ok(n))
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(duration) = self.blocking_flush.take() {
            std::thread::sleep(duration);
        }
        if let Some(duration) = self.flush_delay.take() {
            self.flushing = Some(Box::pin(tokio::time::sleep(duration)));
        }
        if let Some(sleep) = &mut self.flushing {
            if sleep.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            self.flushing = None;
        }
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}
fn video(playout: &mut Playout, seq: u16, timestamp: u32, keyframe: bool, received: Instant) {
    playout.video(
        NativeVideoFrame {
            timestamp_rtp: timestamp,
            keyframe,
            encoded: vec![seq as u8; 4],
        },
        VideoSequenceRange {
            ssrc: 5,
            first: seq,
            last: seq,
        },
        received,
    );
}
async fn seed(playout: &mut Playout, sink: &mut Sink) {
    video(playout, 1, 0, true, now());
    advance(DELAY).await;
    playout
        .drain(sink, &mut None::<Sink>, |_, _| Ok(()), |_| {})
        .await
        .unwrap();
}
async fn records(sink: &Sink) -> Vec<Vec<u8>> {
    let mut bytes = sink.bytes.as_slice();
    let mut frames = Vec::new();
    while let Some(frame) = connect_native_video::read_frame(&mut bytes).await.unwrap() {
        frames.push(frame);
    }
    frames
}

#[tokio::test(start_paused = true)]
async fn delayed_video_flush_expires_ready_audio_before_any_audio_write() {
    let mut playout = Playout::new();
    let mut output = Sink::default();
    seed(&mut playout, &mut output).await;
    playout.audio(
        NativeAudioFrame {
            payload: vec![7; 160],
            timestamp_rtp: 0,
            payload_type: Some(0),
        },
        now(),
    );
    advance(Duration::from_millis(50)).await;
    let video_received = now();
    video(&mut playout, 2, 3000, false, video_received);
    advance(Duration::from_millis(100)).await;
    output.flush_delay = Some(Duration::from_millis(60));
    let mut audio = Some(Sink::default());
    let mut journal = Vec::new();
    playout
        .drain(
            &mut output,
            &mut audio,
            |frame, emitted| {
                journal.push((matches!(frame, Ready::Video(..)), emitted));
                Ok(())
            },
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(
        journal,
        vec![(true, video_received + Duration::from_millis(160))]
    );
    assert!(audio.unwrap().bytes.is_empty());
    assert_eq!(playout.audio.dropped, 1);
    assert_eq!(records(&output).await, vec![vec![1; 4], vec![2; 4]]);
}

#[tokio::test(start_paused = true)]
async fn expired_video_reference_or_keyframe_rejects_dependents_until_new_keyframe() {
    for expired_keyframe in [false, true] {
        let mut playout = Playout::new();
        let mut output = Sink::default();
        seed(&mut playout, &mut output).await;
        video(&mut playout, 2, 3000, expired_keyframe, now());
        advance(Duration::from_millis(50)).await;
        video(&mut playout, 3, 6000, false, now());
        video(&mut playout, 4, 9000, true, now());
        advance(Duration::from_millis(160)).await;
        let mut previews = Vec::new();
        playout
            .drain(
                &mut output,
                &mut None::<Sink>,
                |_, _| Ok(()),
                |frame| previews.push(frame.encoded),
            )
            .await
            .unwrap();
        assert_eq!(playout.video.dropped, 2);
        assert_eq!(previews, vec![vec![4; 4]]);
        assert_eq!(records(&output).await, vec![vec![1; 4], vec![4; 4]]);
    }
}

#[tokio::test(start_paused = true)]
async fn blocked_flush_is_fatal_at_receipt_deadline_without_completed_sample_or_preview() {
    let mut playout = Playout::new();
    let received = now();
    video(&mut playout, 1, 0, true, received);
    video(&mut playout, 2, 3000, false, received);
    advance(DELAY).await;
    let mut sink = Sink {
        flush_delay: Some(Duration::from_millis(100)),
        ..Sink::default()
    };
    let mut journal = 0;
    let mut previews = 0;
    let error = playout
        .drain(
            &mut sink,
            &mut None::<Sink>,
            |_, _| {
                journal += 1;
                Ok(())
            },
            |_| previews += 1,
        )
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("partial output is not resumable"));
    assert_eq!(now(), received + STALE);
    assert_eq!(playout.emission_timeouts, 1);
    assert_eq!((journal, previews), (0, 0));
    assert_eq!(sink.bytes.len(), 16);
    assert_eq!(playout.video.queue.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn partial_ivf_write_timeout_is_fatal_and_does_not_append_a_later_frame() {
    let mut playout = Playout::new();
    let received = now();
    video(&mut playout, 1, 0, true, received);
    video(&mut playout, 2, 3000, false, received);
    advance(DELAY).await;
    let mut sink = Sink {
        partial_bound: Some(4),
        ..Sink::default()
    };
    let mut journal = 0;
    let mut previews = 0;
    assert!(playout
        .drain(
            &mut sink,
            &mut None::<Sink>,
            |_, _| {
                journal += 1;
                Ok(())
            },
            |_| previews += 1
        )
        .await
        .is_err());
    assert_eq!(now(), received + STALE);
    assert_eq!(sink.bytes, 4u32.to_le_bytes());
    assert_eq!((journal, previews), (0, 0));
    assert_eq!(playout.emission_timeouts, 1);
    assert_eq!(playout.video.queue.len(), 1);
}

#[tokio::test]
async fn noncooperative_late_flush_retains_actual_completed_sample_then_fails() {
    let mut playout = Playout::new();
    let received = now() - Duration::from_millis(150);
    video(&mut playout, 1, 0, true, received);
    let mut sink = Sink {
        blocking_flush: Some(Duration::from_millis(75)),
        ..Sink::default()
    };
    let mut journal = Vec::new();
    let mut previews = 0;
    let error = playout
        .drain(
            &mut sink,
            &mut None::<Sink>,
            |_, emitted| {
                journal.push(emitted);
                Ok(())
            },
            |_| previews += 1,
        )
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("Completed codec emission exceeded"));
    assert_eq!(journal.len(), 1);
    assert!(journal[0].duration_since(received) > STALE);
    assert_eq!((playout.late_emissions, previews), (1, 0));
    assert_eq!(records(&sink).await, vec![vec![1; 4]]);
}

#[tokio::test(start_paused = true)]
async fn sink_errors_remain_errors_without_completed_or_preview_claims() {
    let mut playout = Playout::new();
    video(&mut playout, 1, 0, true, now());
    advance(DELAY).await;
    let mut sink = Sink {
        failure: true,
        ..Sink::default()
    };
    let mut journal = 0;
    let mut previews = 0;
    let error = playout
        .drain(
            &mut sink,
            &mut None::<Sink>,
            |_, _| {
                journal += 1;
                Ok(())
            },
            |_| previews += 1,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!((journal, previews), (0, 0));
    assert!(sink.bytes.is_empty());
    assert_eq!((playout.emission_timeouts, playout.late_emissions), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn report_finalization_timeout_preserves_primary_codec_failure() {
    let primary = Err(anyhow::anyhow!("primary partial codec write"));
    let deadline = tokio::time::Instant::now() + Duration::from_millis(40);
    let mut sink = Sink {
        partial_bound: Some(0),
        ..Sink::default()
    };
    let error = crate::report::finish_before(primary, deadline, async {
        sink.write_all(b"observation").await?;
        sink.flush().await?;
        Ok(())
    })
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("primary partial codec write"));
    assert!(format!("{error:#}").contains("Observation finalization"));
    assert_eq!(tokio::time::Instant::now(), deadline);
    assert!(sink.bytes.is_empty());
}

#[tokio::test]
async fn completed_late_report_cannot_claim_successful_teardown() {
    let mut sink = Sink {
        blocking_flush: Some(Duration::from_millis(75)),
        ..Sink::default()
    };
    let deadline = tokio::time::Instant::now() + Duration::from_millis(50);
    let error = crate::report::finish_before(Ok(()), deadline, async {
        sink.write_all(b"completed observation").await?;
        sink.flush().await?;
        Ok(())
    })
    .await
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("completed after the teardown deadline"));
    assert_eq!(sink.bytes, b"completed observation");
}
