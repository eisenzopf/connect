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
    pub sip: bool,
    pub providers: Vec<String>,
    pub vapi_audio: bool,
    pub browser: Option<BrowserRoute>,
}
/// A browser binding is visible only to the exact configured route credential.
#[derive(Clone)]
pub struct BrowserRoute {
    pub credential_id: String,
    pub binding: BrowserBinding,
}

pub(crate) fn browser_binding(
    c: &Connection,
    actor: &Actor,
    runtime: &DiscoveryRuntime,
) -> Result<BrowserBinding> {
    let route = runtime
        .browser
        .as_ref()
        .ok_or_else(|| Error::new("unsupported-operation", "Browser voice is not configured"))?;
    if route.credential_id != actor.credential_id {
        return Err(Error::forbidden());
    }
    let conversation = authorized_conversation(c, actor, &route.binding.conversation_id)?;
    if conversation.closed {
        return Err(Error::new(
            "conversation-closed",
            "Browser route conversation is closed",
        ));
    }
    if !actor.permits_recipient(&route.binding.participant_id) {
        return Err(Error::forbidden());
    }
    Ok(route.binding.clone())
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
            if browser.credential_id == actor.credential_id
                && browser.binding.conversation_id == cid
                && browser.binding.participant_id == identity.id
            {
                add(
                    "browser",
                    "webrtc",
                    &["audio"],
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
        },
    })
}
