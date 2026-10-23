//! Raw artifact-byte routes for the loopback HTTP SDK.
use super::http::{authenticated_credential, error, HttpState};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_protocol::{Error, Operation, Outcome, PROFILE};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PutQuery {
    request_id: String,
    offset: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GetQuery {
    offset: u64,
    limit: u32,
}

pub(super) async fn put_chunk(
    State(state): State<HttpState>,
    Path((conversation_id, artifact_id)): Path<(String, String)>,
    Query(query): Query<PutQuery>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return capacity_error();
    };
    if bytes.is_empty() || bytes.len() > state.service.store.limits().artifact_chunk_bytes as usize
    {
        return error(
            StatusCode::BAD_REQUEST,
            Error::new(
                "invalid-request",
                "Artifact chunk is outside configured bounds",
            ),
        );
    }
    let credential = match authenticated_credential(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let result = state
        .service
        .execute(
            credential,
            Operation::ArtifactAppend {
                request_id: query.request_id,
                conversation_id,
                artifact_id,
                offset: query.offset,
                data: STANDARD.encode(bytes),
            },
        )
        .await;
    let status = match &result {
        Err(value) if value.code == "forbidden" => StatusCode::FORBIDDEN,
        Err(value) if matches!(value.code.as_str(), "invalid-request" | "conflict") => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::OK,
    };
    (
        status,
        Json(connect_protocol::Response {
            profile: PROFILE.into(),
            result,
        }),
    )
        .into_response()
}

pub(super) async fn get_chunk(
    State(state): State<HttpState>,
    Path((conversation_id, artifact_id)): Path<(String, String)>,
    Query(query): Query<GetQuery>,
    headers: HeaderMap,
) -> Response {
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return capacity_error();
    };
    let credential = match authenticated_credential(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .service
        .execute(
            credential,
            Operation::ArtifactGet {
                conversation_id,
                artifact_id,
                offset: query.offset,
                limit: query.limit,
            },
        )
        .await
    {
        Ok(Outcome::ArtifactChunk(chunk)) => binary_chunk(chunk),
        Ok(_) => error(StatusCode::SERVICE_UNAVAILABLE, Error::storage()),
        Err(value) => {
            let status = match value.code.as_str() {
                "forbidden" => StatusCode::FORBIDDEN,
                "invalid-request" => StatusCode::BAD_REQUEST,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            error(status, value)
        }
    }
}

fn binary_chunk(chunk: connect_protocol::ArtifactChunk) -> Response {
    let Ok(bytes) = STANDARD.decode(chunk.data) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, Error::storage());
    };
    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&chunk.offset.to_string()) {
        response.headers_mut().insert("x-connect-offset", value);
    }
    response.headers_mut().insert(
        "x-connect-eof",
        HeaderValue::from_static(if chunk.eof { "true" } else { "false" }),
    );
    response
}

fn capacity_error() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        Error::new("capacity-exceeded", "HTTP concurrency limit reached"),
    )
}

/// These raw-byte routes accept ordinary credentials. Delegated clients redeem
/// the same link through their Task Invoke facade, retaining grant provenance.
pub(super) async fn get_link_chunk(
    State(state): State<HttpState>,
    Path((conversation_id, artifact_id, token)): Path<(String, String, String)>,
    Query(query): Query<GetQuery>,
    headers: HeaderMap,
) -> Response {
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return capacity_error();
    };
    let credential = match authenticated_credential(&state, &headers).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let link = format!("/v1/conversations/{conversation_id}/artifacts/{artifact_id}/links/{token}");
    match state
        .service
        .execute(
            credential,
            Operation::ArtifactLinkGet {
                conversation_id,
                artifact_id,
                link,
                offset: query.offset,
                limit: query.limit,
            },
        )
        .await
    {
        Ok(Outcome::ArtifactChunk(value)) => binary_chunk(value),
        Ok(_) => error(StatusCode::SERVICE_UNAVAILABLE, Error::storage()),
        Err(value) => error(
            if value.code == "forbidden" {
                StatusCode::FORBIDDEN
            } else if value.code == "invalid-request" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            value,
        ),
    }
}
pub(super) async fn put_link_chunk(
    State(state): State<HttpState>,
    Path((conversation_id, artifact_id, token)): Path<(String, String, String)>,
    Query(query): Query<PutQuery>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return capacity_error();
    };
    let credential = match authenticated_credential(&state, &headers).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if bytes.is_empty() || bytes.len() > state.service.store.limits().artifact_chunk_bytes as usize
    {
        return error(
            StatusCode::BAD_REQUEST,
            Error::new(
                "invalid-request",
                "Artifact chunk is outside configured bounds",
            ),
        );
    }
    let link = format!("/v1/conversations/{conversation_id}/artifacts/{artifact_id}/links/{token}");
    let result = state
        .service
        .execute(
            credential,
            Operation::ArtifactLinkAppend {
                request_id: query.request_id,
                conversation_id,
                artifact_id,
                link,
                offset: query.offset,
                data: STANDARD.encode(bytes),
            },
        )
        .await;
    let status = match &result {
        Err(v) if v.code == "forbidden" => StatusCode::FORBIDDEN,
        Err(v) if matches!(v.code.as_str(), "invalid-request" | "conflict") => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::OK,
    };
    (
        status,
        Json(connect_protocol::Response {
            profile: PROFILE.into(),
            result,
        }),
    )
        .into_response()
}
