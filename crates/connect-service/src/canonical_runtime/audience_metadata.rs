//! Historical descriptor changes captured by the actual atomic Core commit.
use connect_protocol::{
    native_audience::{AudienceMetadata, AudienceStreamState},
    Error, Result,
};
use rvoip_core::{
    staged_streams::audience::AudioAudienceCommit,
    stream_lifecycle::{LogicalStreamState, Medium},
};

pub(crate) fn capture_audience_metadata(commit: &AudioAudienceCommit) -> Result<AudienceMetadata> {
    commit
        .connections()
        .map(|id| {
            let streams = commit
                .captured_gates(id)
                .ok_or_else(Error::storage)?
                .iter()
                .map(|stream| {
                    if stream.negotiated.requested.medium != Medium::Audio {
                        return Err(Error::new(
                            "invalid-observation",
                            "Unexpected non-audio audience gate",
                        ));
                    }
                    Ok(AudienceStreamState {
                        stream_id: stream.negotiated.requested.logical_id.as_str().into(),
                        stream_generation: stream.generation,
                        state: match stream.state {
                            LogicalStreamState::Active => "active",
                            LogicalStreamState::Paused => "paused",
                            LogicalStreamState::Ended => "ended",
                        }
                        .into(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((id.as_str().into(), streams))
        })
        .collect()
}
