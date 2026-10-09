use crate::{now, Actor};
use connect_protocol::{Availability, AvailabilityState, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn set(
    c: &Connection,
    actor: &Actor,
    state: &AvailabilityState,
    preferred_route: Option<&str>,
    expires_at: i64,
) -> Result<Availability> {
    let observed_at = now();
    if expires_at <= observed_at || expires_at > observed_at + 24 * 60 * 60 {
        return Err(Error::new(
            "invalid-request",
            "Availability expiry must be within the next 24 hours",
        ));
    }
    if preferred_route.is_some_and(|route| {
        route.is_empty()
            || route.len() > 128
            || !route
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
    }) {
        return Err(Error::new("invalid-request", "Preferred route is invalid"));
    }
    let state = match state {
        AvailabilityState::Available => "available",
        AvailabilityState::Unavailable => "unavailable",
    };
    c.execute(
        "INSERT INTO availability(identity_id,state,preferred_route,expires_at,observed_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(identity_id) DO UPDATE SET state=excluded.state,preferred_route=excluded.preferred_route,expires_at=excluded.expires_at,observed_at=excluded.observed_at",
        params![actor.identity_id, state, preferred_route, expires_at, observed_at],
    )
    .map_err(|_| Error::storage())?;
    get(c, &actor.identity_id)
}

pub(crate) fn get(c: &Connection, identity_id: &str) -> Result<Availability> {
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1)",
            [identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !exists {
        return Err(Error::forbidden());
    }
    let row: Option<(String, Option<String>, i64, i64)> = c
        .query_row(
            "SELECT state,preferred_route,expires_at,observed_at FROM availability WHERE identity_id=?1",
            [identity_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    match row {
        Some((state, preferred_route, expires_at, observed_at)) if expires_at > now() => {
            Ok(Availability {
                identity_id: identity_id.into(),
                state,
                preferred_route,
                expires_at: Some(expires_at),
                observed_at: Some(observed_at),
            })
        }
        _ => Ok(Availability {
            identity_id: identity_id.into(),
            state: "unknown".into(),
            preferred_route: None,
            expires_at: None,
            observed_at: None,
        }),
    }
}
