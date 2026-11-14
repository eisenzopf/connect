//! Fixture-only replay of an already confirmed immutable native Start receipt.
//! No initial submission, uncertain outcome, or terminal authorization refusal retries.
use connect_client::canonical::{CanonicalClient, CommandScope, ReplyBatch};
use connect_client::{ClientError, Result};
use serde_json::Value;
use tokio::time::{sleep_until, timeout_at, Duration, Instant};

enum Decision {
    Done,
    Wait(Instant),
    Stop(ClientError),
}
fn decision(reply: Result<ReplyBatch>, now: Instant, until: Instant) -> Decision {
    match reply.and_then(|batch| batch.ack().map(|_| ())) {
        Ok(()) if now < until => Decision::Done,
        Ok(()) => Decision::Stop(ClientError::Timeout),
        Err(ClientError::Application(problem)) if problem.code == "storage-unavailable" => {
            if now >= until {
                Decision::Stop(ClientError::Timeout)
            } else {
                Decision::Wait(until.min(now + Duration::from_millis(20)))
            }
        }
        Err(error) => Decision::Stop(error),
    }
}
pub(super) async fn acknowledge_confirmed_start(
    client: &mut CanonicalClient,
    frozen_payload: Value,
    frozen_scope: CommandScope,
    original_cutoff: Instant,
) -> Result<()> {
    // Each request uses the same logical operation and payload. CanonicalClient
    // creates a fresh signed wire envelope and the server rechecks current
    // authority. Only the already-confirmed Start replay calls this helper.
    loop {
        if Instant::now() >= original_cutoff {
            return Err(ClientError::Timeout);
        }
        let reply = timeout_at(
            original_cutoff,
            client.request_value(
                "connection.offer",
                frozen_payload.clone(),
                frozen_scope.clone(),
            ),
        )
        .await
        .map_err(|_| ClientError::Timeout)?;
        match decision(reply, Instant::now(), original_cutoff) {
            Decision::Done => return Ok(()),
            Decision::Stop(error) => return Err(error),
            Decision::Wait(next) => {
                timeout_at(original_cutoff, sleep_until(next))
                    .await
                    .map_err(|_| ClientError::Timeout)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_client::canonical::Envelope;
    use serde_json::json;

    fn batch(code: Option<u64>) -> ReplyBatch {
        let (kind, payload) = match code {
            Some(code) => (
                "error",
                json!({"code":code,"category":"authorization","reason":"authority_unavailable"}),
            ),
            None => ("ack", json!({})),
        };
        ReplyBatch {
            request_id: "same-replay-wire".into(),
            frames: vec![Envelope::from_value(json!({"v":2,"type":kind,
                "id":"signed-result","ts":chrono::Utc::now(),
                "cid":"room","sid":"native-session","connid":"candidate",
                "operation_id":"same-original-replacement","in_reply_to":"same-replay-wire",
                "payload":payload}))
            .unwrap()],
        }
    }
    #[test]
    fn confirmed_start_replay_only_retries_signed_transient_before_original_cutoff() {
        let now = Instant::now();
        let until = now + Duration::from_millis(25);
        // The real public ReplyBatch ACK classifier maps signed503 to the same
        // transient as a request-level Application error. Neither is success.
        for result in [
            Err(ClientError::Application(connect_protocol::Error::new(
                "storage-unavailable",
                "busy",
            ))),
            Ok(batch(Some(503))),
        ] {
            assert!(
                matches!(decision(result, now, until), Decision::Wait(at) if at == now + Duration::from_millis(20))
            );
        }
        assert!(matches!(
            decision(Ok(batch(None)), now, until),
            Decision::Done
        ));
        for observed in [until, until + Duration::from_nanos(1)] {
            assert!(matches!(
                decision(Ok(batch(None)), observed, until),
                Decision::Stop(ClientError::Timeout)
            ));
        }
        // Repeated refusals never mint another25ms budget. The final backoff
        // ends at the SAME original cutoff, and expiration permits no request.
        assert!(
            matches!(decision(Ok(batch(Some(503))), now + Duration::from_millis(21), until), Decision::Wait(at) if at == until)
        );
        assert!(matches!(
            decision(Ok(batch(Some(503))), until, until),
            Decision::Stop(ClientError::Timeout)
        ));
        for code in [400, 401, 403, 409, 429, 488, 501] {
            assert!(matches!(
                decision(Ok(batch(Some(code))), now, until),
                Decision::Stop(ClientError::Application(_))
            ));
        }
        for error in [
            ClientError::Timeout,
            ClientError::Transport,
            ClientError::Cancelled,
            ClientError::Authentication,
            ClientError::Protocol,
            ClientError::Application(connect_protocol::Error::new(
                "authority-busy",
                "not the explicit public503",
            )),
        ] {
            assert!(matches!(
                decision(Err(error), now, until),
                Decision::Stop(_)
            ));
        }
    }
}
