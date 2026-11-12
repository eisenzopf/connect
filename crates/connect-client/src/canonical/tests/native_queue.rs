//! Local transport custody only: no server media grant or decoded QoE claim.
use super::*;
use std::{future::Future, task::Poll};
async fn joined_catch<F: Future>(future: F) -> std::thread::Result<F::Output> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(panic) => Poll::Ready(Err(panic)),
        }
    })
    .await
}
fn packet(pt: u8, sequence: u32, size: usize) -> Vec<u8> {
    let mut bytes = vec![1, 0, 0, 7];
    bytes.extend_from_slice(&sequence.to_be_bytes());
    bytes.extend_from_slice(&[0x80, pt]);
    bytes.extend_from_slice(&(sequence as u16).to_be_bytes());
    bytes.extend_from_slice(&(sequence * 960).to_be_bytes());
    bytes.extend_from_slice(&41u32.to_be_bytes());
    bytes.extend(std::iter::repeat_n(0x55, size));
    bytes
}
async fn exercise(raw_owner: bool) {
    let (mut client, server, _) =
        controlled(Fault::NativeDatagrams(native_media::Traffic::QueuePressure)).await;
    let mut server = server;
    let work = joined_catch(async {
        client.request(query(), scope()).await?;
        let audio = packet(111, 1, 80);
        let media = if raw_owner {
            Some(client.claim_native_datagrams()?)
        } else {
            None
        };
        let before = client.connection.datagram_send_buffer_space();
        let first = match &media {
            Some(owner) => owner.send(&audio),
            None => client.send_native_media_datagram(&audio),
        };
        let mut accepted = 0usize;
        let mut refused = Vec::new();
        // Current-thread runtime + NO await: Quinn's driver cannot drain this
        // burst while its actual atomic outgoing queue is being exercised.
        for index in 2..130 {
            let video = packet(96, index, 1000);
            match match &media {
                Some(owner) => owner.send(&video),
                None => client.send_native_media_datagram(&video),
            } {
                Ok(()) => accepted += 1,
                Err(error) => refused.push(error.to_string()),
            }
        }
        let after = client.connection.datagram_send_buffer_space();
        drop(media);
        let summary =
            tokio::time::timeout(Duration::from_secs(3), client.connection.read_datagram())
                .await
                .map_err(|_| ClientError::Timeout)?
                .map_err(|_| ClientError::Transport)?;
        let summary: Value = serde_json::from_slice(&summary).map_err(|_| ClientError::Protocol)?;
        Ok::<_, ClientError>((audio, first, before, after, accepted, refused, summary))
    })
    .await;
    // No intended old-condition assertion precedes original owner closure.
    let closed = joined_catch(client.shutdown()).await;
    let joined = tokio::time::timeout(Duration::from_secs(3), &mut server).await;
    let joined = match joined {
        Ok(result) => Ok(result),
        Err(_) => {
            server.abort();
            let forced = server.await;
            eprintln!("original controlled server required forced join: {forced:?}");
            Err("original controlled server timed out; forced join cannot qualify")
        }
    };
    let work = match work {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    let (audio, first, before, after, accepted, refused, summary) = work.unwrap();
    closed.unwrap().unwrap();
    joined
        .expect("original controlled server must join naturally")
        .unwrap();
    first.unwrap();
    assert!(
        before <= 64 * 1024 && before > 60 * 1024,
        "exact old64KiB queue setup"
    );
    assert!(
        after < before && accepted > 0,
        "actual native datagrams never entered outgoing queue"
    );
    assert!(
        refused.iter().all(|e| e.contains("queue is full")),
        "unexpected pre-submission failure: {refused:?}"
    );
    assert!(summary["datagrams"].as_u64().unwrap() > 0);
    assert_eq!(summary["audio"],json!([audio]),
        "later video evicted an already accepted original audio datagram from the local native queue");
}
#[tokio::test(flavor = "current_thread")]
async fn canonical_media_video_burst_does_not_evict_accepted_original_audio() {
    exercise(false).await;
}
#[tokio::test(flavor = "current_thread")]
async fn raw_native_owner_video_burst_does_not_evict_accepted_original_audio() {
    exercise(true).await;
}
