//! One explicitly configured HTTPS origin and immediate proxy hop.
use super::http_record_io::Custody;
use crate::hosting::HttpsListenerConfig;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

pub(super) struct Policy {
    pub config: HttpsListenerConfig,
    pub authority: String,
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        None
    } else {
        Some(value)
    }
}
fn refused() -> Response {
    let error = connect_protocol::Error::forbidden();
    (
        StatusCode::FORBIDDEN,
        [(header::CACHE_CONTROL, "no-store")],
        Json(connect_protocol::canonical_http::Rejection {
            profile: "connect/2".into(),
            problem: connect_protocol::canonical_http::Problem::from_error(&error),
        }),
    )
        .into_response()
}
pub(super) async fn admit(
    State(policy): State<Arc<Policy>>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    if single(headers, "host") != Some(policy.authority.as_str())
        || request
            .uri()
            .authority()
            .is_some_and(|a| a.as_str() != policy.authority)
        || request.uri().scheme_str().is_some_and(|s| s != "https")
        || headers.contains_key(header::UPGRADE)
    {
        return refused();
    }
    if headers.contains_key(header::ORIGIN)
        && !single(headers, "origin")
            .is_some_and(|origin| policy.config.allowed_origins.iter().any(|o| o == origin))
    {
        return refused();
    }
    // These never become credentials, actor IDs, assurance or provider context,
    // including when sent by a configured proxy.
    for name in [
        "forwarded",
        "x-forwarded-user",
        "x-forwarded-client-cert",
        "x-remote-user",
        "x-user-id",
        "x-authenticated-user",
        "x-auth-request-user",
        "x-ssl-client-verify",
        "x-ssl-client-dn",
        "x-call-id",
        "x-chat-id",
        "x-forwarded-port",
        "x-forwarded-prefix",
    ] {
        if headers.contains_key(name) {
            return refused();
        }
    }
    const FORWARDING: [&str; 3] = ["x-forwarded-proto", "x-forwarded-host", "x-forwarded-for"];
    if FORWARDING.iter().any(|name| headers.contains_key(*name)) {
        let peer = request
            .extensions()
            .get::<ConnectInfo<Custody>>()
            .map(|c| c.0.peer_address());
        if !peer.is_some_and(|address| policy.config.trusted_proxies.contains(&address.ip()))
            || single(headers, FORWARDING[0]) != Some("https")
            || single(headers, FORWARDING[1]) != Some(policy.authority.as_str())
            || !single(headers, FORWARDING[2]).is_some_and(|ip| {
                ip.len() <= 45
                    && ip
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| !ip.is_unspecified() && !ip.is_multicast())
            })
        {
            return refused();
        }
        for name in FORWARDING {
            request.headers_mut().remove(name);
        }
    }
    let mut result = next.run(request).await;
    result.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    result
}
