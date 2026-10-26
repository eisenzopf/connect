//! Compile-only application outline against the real public crate.
//! Receipts describe recorded effects and historical parameters, never flow readiness.
use connect_client::sdk::{
    Connection, ConnectionUpdate, ConnectionUpdateReceipt, Medium, MutationOptions, NonEmpty,
    ParticipantId, RequestOptions, Result, StreamId, StreamRevision, StreamSelectionReceipt,
    StreamSelector,
};

/// All three original mutation IDs and complete inputs must be saved before IO.
pub async fn observe_lifecycle(
    connection: &Connection,
    from: ParticipantId,
    source: StreamId,
    expected: StreamRevision,
    mutations: [MutationOptions; 3],
    wait: RequestOptions,
) -> Result<(ConnectionUpdateReceipt, StreamSelectionReceipt, Connection)> {
    let [update, selection, end] = mutations;
    let update: ConnectionUpdateReceipt = connection
        .update(
            ConnectionUpdate::CodecRenegotiate {
                streams: Some(NonEmpty {
                    first: source,
                    rest: vec![],
                }),
                codec_preferences: NonEmpty {
                    first: "pcmu".into(),
                    rest: vec![],
                },
            },
            expected,
            update,
        )
        .await?
        .wait(wait.clone())
        .await?
        .value;
    let selection: StreamSelectionReceipt = connection
        .subscribe(
            NonEmpty {
                first: StreamSelector::Participant {
                    participant_id: from,
                    kinds: Some(NonEmpty {
                        first: Medium::Audio,
                        rest: vec![],
                    }),
                },
                rest: vec![],
            },
            update.stream_revision,
            selection,
        )
        .await?
        .wait(wait.clone())
        .await?
        .value;
    // None means unavailable capture. A known empty capture remains Some(0).
    let _observed_binding_count = selection.bindings.as_ref().map(Vec::len);
    let ended = connection
        .end(
            1000,
            "Application completed".into(),
            Some(selection.stream_revision),
            end,
        )
        .await?
        .wait(wait)
        .await?
        .value;
    Ok((update, selection, ended))
}
fn main() {}
