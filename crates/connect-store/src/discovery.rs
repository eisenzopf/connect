//! Read-only discovery combines durable routes with explicit runtime configuration.
use crate::{conversations::authorized_conversation, Actor};
use connect_protocol::{
    BrowserBinding, Capabilities, CapabilityLimits, Error, Limits, Outcome, Result, RouteCapability,
};
use rusqlite::{params, Connection};

/// Immutable runtime inputs. No secrets, sockets or provider credentials enter the store.
#[derive(Clone, Default)]
pub struct DiscoveryRuntime {
    pub native_audio: bool,
    pub native_video: bool,
    pub sip: bool,
    pub providers: Vec<String>,
    pub vapi_audio: bool,
    pub browser: Option<BrowserRoute>,
}
/// Legacy audio defaults to one configured credential. Selected AV is restricted
/// to the configured pair and their currently authorized accepted Session.
#[derive(Clone)]
pub struct BrowserRoute {
    pub credential_id: String,
    pub binding: BrowserBinding,
}

pub(crate) fn browser_binding(
    c: &Connection,
    actor: &Actor,
    runtime: &DiscoveryRuntime,
    conversation_id: Option<&str>,
    session_id: Option<&str>,
) -> Result<BrowserBinding> {
    let route = runtime
        .browser
        .as_ref()
        .ok_or_else(|| Error::new("unsupported-operation", "Browser voice is not configured"))?;
    match (conversation_id, session_id) {
        (Some(cid), Some(sid)) => {
            let session = selected_browser_video_session(c, actor, route, cid, sid)?;
            return Ok(BrowserBinding {
                conversation_id: session.conversation_id,
                participant_id: if actor.identity_id == session.initiator_id {
                    session.participant_id
                } else {
                    session.initiator_id
                },
                session_id: Some(session.id),
                signaling_url: route.binding.signaling_url.clone(),
            });
        }
        (None, None) => {}
        _ => {
            return Err(Error::new(
                "invalid-request",
                "Browser conversation and Session must be selected together",
            ))
        }
    }
    let video_session = route
        .binding
        .session_id
        .as_deref()
        .map(|id| crate::sessions::get(c, &route.binding.conversation_id, id))
        .transpose()?;
    let video = video_session
        .as_ref()
        .is_some_and(|session| session.media == ["audio", "video"]);
    if video {
        let configured = crate::identity::actor_from_connection(c, &route.credential_id)?;
        crate::sessions::authorize_binding(
            c,
            &configured,
            route
                .binding
                .session_id
                .as_deref()
                .ok_or_else(Error::forbidden)?,
        )?;
        crate::sessions::authorize_binding(
            c,
            actor,
            route
                .binding
                .session_id
                .as_deref()
                .ok_or_else(Error::forbidden)?,
        )?;
    } else if route.credential_id != actor.credential_id {
        return Err(Error::forbidden());
    }
    let conversation = authorized_conversation(c, actor, &route.binding.conversation_id)?;
    if conversation.closed {
        return Err(Error::new(
            "conversation-closed",
            "Browser route conversation is closed",
        ));
    }
    let mut binding = route.binding.clone();
    if let Some(session) = video_session.filter(|_| video) {
        if session.participant_id != binding.participant_id {
            return Err(Error::forbidden());
        }
        binding.participant_id = if actor.identity_id == session.initiator_id {
            session.participant_id
        } else {
            session.initiator_id
        };
    }
    if !actor.permits_recipient(&binding.participant_id) {
        return Err(Error::forbidden());
    }
    Ok(binding)
}

fn valid_selection_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}
/// Configuration names the pair; caller-supplied IDs can only select one of
/// their accepted Sessions. They never establish authority by themselves.
pub(crate) fn selected_browser_video_session(
    c: &Connection,
    actor: &Actor,
    route: &BrowserRoute,
    cid: &str,
    sid: &str,
) -> Result<connect_protocol::Session> {
    if !valid_selection_id(cid) || !valid_selection_id(sid) {
        return Err(Error::new(
            "invalid-request",
            "Browser Session selection is invalid",
        ));
    }
    let configured = crate::identity::actor_from_connection(c, &route.credential_id)?;
    let configured_session = crate::sessions::authorize_binding(c, &configured, sid)?;
    let session = crate::sessions::authorize_binding(c, actor, sid)?;
    let peer = &route.binding.participant_id;
    let pair = (session.initiator_id == configured.identity_id && session.participant_id == *peer)
        || (session.participant_id == configured.identity_id && session.initiator_id == *peer);
    if !pair
        || configured.identity_id == *peer
        || session.conversation_id != cid
        || configured_session.id != session.id
        || session.media != ["audio", "video"]
    {
        return Err(Error::forbidden());
    }
    let other = if actor.identity_id == session.initiator_id {
        &session.participant_id
    } else {
        &session.initiator_id
    };
    if !actor.permits_recipient(other) || !configured.permits_recipient(peer) {
        return Err(Error::forbidden());
    }
    Ok(session)
}
fn configured_video_counterpart(
    c: &Connection,
    actor: &Actor,
    route: &BrowserRoute,
    cid: &str,
) -> Result<String> {
    let configured = crate::identity::actor_from_connection(c, &route.credential_id)?;
    configured.require("session_get")?;
    let conversation = authorized_conversation(c, &configured, cid)?;
    let peer = &route.binding.participant_id;
    if conversation.closed
        || configured.identity_id == *peer
        || !conversation.participants.contains(peer)
        || !configured.permits_recipient(peer)
    {
        return Err(Error::forbidden());
    }
    let other = if actor.identity_id == configured.identity_id {
        peer.clone()
    } else if actor.identity_id == *peer {
        configured.identity_id
    } else {
        return Err(Error::forbidden());
    };
    if !actor.permits_recipient(&other) {
        return Err(Error::forbidden());
    }
    Ok(other)
}
impl crate::Store {
    /// Revalidate current participants/configured route authority for a
    /// selected browser AV leg before projecting it into live media.
    pub fn authorize_selected_browser_video(
        &self,
        credential_id: &str,
        route: &BrowserRoute,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<crate::RoutedInboundSession> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential_id)?;
        let session =
            selected_browser_video_session(&tx, &actor, route, conversation_id, session_id)?;
        Ok(crate::RoutedInboundSession {
            session,
            credential_id: actor.credential_id,
            caller_identity_id: actor.identity_id,
        })
    }
}

pub(crate) fn capabilities(
    c: &Connection,
    actor: &Actor,
    address: &str,
    cid: Option<&str>,
    runtime: &DiscoveryRuntime,
    limits: &Limits,
) -> Result<Capabilities> {
    let Outcome::Identity(identity) = crate::conversation_queries::resolve(c, actor, address)?
    else {
        return Err(Error::storage());
    };
    if let Some(cid) = cid {
        let conversation = authorized_conversation(c, actor, cid)?;
        if !conversation.participants.contains(&identity.id) {
            return Err(Error::forbidden());
        }
        if conversation.closed {
            return Err(Error::new(
                "conversation-closed",
                "Capability target conversation is closed",
            ));
        }
    }
    let mut routes = Vec::new();
    let mut add =
        |id: &str, transport: &str, media: &[&str], operations: &[&str], configured: bool| {
            let allowed: Vec<String> = operations
                .iter()
                .filter(|op| actor.permits(op))
                .map(|s| s.to_string())
                .collect();
            if !allowed.is_empty() {
                routes.push(RouteCapability {
                    id: id.into(),
                    transport: transport.into(),
                    media: media.iter().map(|s| s.to_string()).collect(),
                    operations: allowed,
                    readiness: if configured {
                        "configured"
                    } else {
                        "unconfigured"
                    }
                    .into(),
                });
            }
        };
    add(
        "native-messaging",
        "uctp-quic",
        &["messages"],
        &["send", "history"],
        true,
    );
    add(
        "reliable-artifacts",
        "uctp-quic/http",
        &["files"],
        &[
            "artifact_begin",
            "artifact_append",
            "artifact_finish",
            "artifact_get",
            "artifact_delete",
        ],
        true,
    );
    if actor.permits("session_invite") {
        add(
            "native-audio",
            "uctp-quic",
            &["audio"],
            &["session_invite", "session_get", "session_end"],
            runtime.native_audio,
        );
        add(
            "native-video",
            "uctp-quic",
            &["audio", "video"],
            &["session_invite", "session_get", "session_end"],
            runtime.native_video,
        );
    }
    if let Some(cid) = cid {
        let mut st = c.prepare("SELECT id,connector FROM connector_routes WHERE enabled=1 AND conversation_id=?1 AND identity_id=?2 ORDER BY id").map_err(|_|Error::storage())?;
        let rows = st
            .query_map(params![cid, identity.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|_| Error::storage())?;
        for row in rows {
            let (id, provider) = row.map_err(|_| Error::storage())?;
            add(
                &id,
                &provider,
                &["messages"],
                &["send", "provider_status"],
                runtime.providers.contains(&provider),
            );
        }
        let mut st = c.prepare("SELECT id FROM voice_routes WHERE enabled=1 AND conversation_id=?1 AND identity_id=?2 ORDER BY id").map_err(|_|Error::storage())?;
        let rows = st
            .query_map(params![cid, identity.id], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?;
        for row in rows {
            add(
                &row.map_err(|_| Error::storage())?,
                "vapi-pstn-control",
                &["audio"],
                &["call_start", "call_status", "call_end"],
                runtime.providers.iter().any(|p| p == "vapi"),
            );
        }
        if crate::session_routes::vapi_assistant(c, cid, &identity.id)?.is_some() {
            add(
                "vapi-duplex",
                "uctp-quic/vapi-websocket",
                &["audio"],
                &["session_invite", "session_get", "session_end"],
                runtime.vapi_audio,
            );
        }
        if runtime.sip {
            add(
                "sip",
                "sip-rtp",
                &["audio"],
                &["sip_dial", "sip_status", "sip_end"],
                true,
            );
        }
        if let Some(browser) = &runtime.browser {
            let video = configured_video_counterpart(c, actor, browser, cid)
                .is_ok_and(|peer| peer == identity.id);
            let audio = browser.binding.conversation_id == cid
                && browser_binding(c, actor, runtime, None, None)
                    .is_ok_and(|binding| binding.participant_id == identity.id);
            if video || audio {
                add(
                    "browser",
                    "webrtc",
                    if video {
                        &["audio", "video"]
                    } else {
                        &["audio"]
                    },
                    &["session_invite", "session_get", "session_end"],
                    true,
                );
            }
        }
    }
    let availability = if actor.permits("availability_get") {
        Some(crate::availability::get(c, &identity.id)?)
    } else {
        None
    };
    Ok(Capabilities {
        extensions: vec![connect_protocol::tasks::TASK_PROFILE.into()],
        identity_id: identity.id,
        conversation_id: cid.map(str::to_owned),
        routes,
        availability,
        limits: CapabilityLimits {
            message_bytes: limits.message_bytes,
            artifact_bytes: limits.artifact_file_bytes,
            artifact_chunk_bytes: limits.artifact_chunk_bytes,
            page_size: limits.page_size,
            audio_frame_ms: 20,
            audio_codec: "PCMU/8000".into(),
            video_profile: runtime
                .native_video
                .then(connect_protocol::video_profile::VideoProfile::default),
        },
    })
}
