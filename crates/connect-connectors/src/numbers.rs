//! Explicit purchases: a persisted customer reference owns order reconciliation.
use crate::Telnyx;
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, io::Write, path::Path};
#[derive(Serialize, Deserialize)]
struct Purchase {
    number: String,
    connection_id: String,
    reference: String,
    order_id: Option<String>,
    order: Option<Value>,
}
fn save(path: &Path, value: &Purchase) -> Result<()> {
    let temp = path.with_extension("pending.json");
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
pub async fn search(country: &str, area: Option<&str>) -> Result<Value> {
    let t = Telnyx::from_env()?;
    search_with_client(&t, country, area).await
}
/// Read-only inventory using the host-selected protected provider client.
pub async fn search_with_client(t: &Telnyx, country: &str, area: Option<&str>) -> Result<Value> {
    ensure!(
        country.len() == 2 && country.chars().all(|c| c.is_ascii_uppercase()),
        "Use a two-letter country code"
    );
    if let Some(area) = area {
        ensure!(
            !area.is_empty() && area.chars().all(|c| c.is_ascii_digit()),
            "Invalid area code"
        );
    }
    let mut request = t
        .client
        .get::<Value>("/available_phone_numbers")
        .query_pair("filter[country_code]", country)
        .query_pair("filter[features]", "sms")
        .query_pair("filter[phone_number_type]", "local")
        .query_pair("filter[limit]", "5");
    if let Some(area) = area {
        request = request.query_pair("filter[national_destination_code]", area);
    }
    request.await.map_err(|e| {
        anyhow::anyhow!(
            "Number inventory search failed (status {:?}, codes {:?})",
            e.status().map(|s| s.as_u16()),
            e.api_errors()
                .iter()
                .map(|r| r.code.as_deref())
                .collect::<Vec<_>>()
        )
    })
}
pub async fn order(number: &str, connection: &str, state: &Path) -> Result<()> {
    let t = Telnyx::from_env()?;
    order_with_client(&t, number, connection, state).await
}
/// Explicit operator purchase. Never invoked by server startup or offline checks.
pub async fn order_with_client(
    t: &Telnyx,
    number: &str,
    connection: &str,
    state: &Path,
) -> Result<()> {
    ensure!(
        number.starts_with('+')
            && (8..=16).contains(&number.len())
            && number[1..].chars().all(|c| c.is_ascii_digit()),
        "Use a valid E.164 number from recent inventory"
    );
    ensure!(
        !connection.is_empty() && connection.chars().all(|c| c.is_ascii_digit()),
        "Invalid SIP connection ID"
    );
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(state.with_extension("lock"))?;
    lock.try_lock_exclusive()
        .context("Another number order is running")?;
    let existing = state.exists();
    let mut purchase: Purchase = if existing {
        serde_json::from_slice(&fs::read(state)?)?
    } else {
        Purchase {
            number: number.into(),
            connection_id: connection.into(),
            reference: format!("connect-test-{}", uuid::Uuid::new_v4().simple()),
            order_id: None,
            order: None,
        }
    };
    ensure!(
        purchase.number == number && purchase.connection_id == connection,
        "Order inputs changed; keep original state to resolve uncertainty"
    );
    if purchase.order_id.is_none() {
        let response: Value = if existing {
            let rows: Value = t
                .client
                .get("/number_orders")
                .query_pair("filter[customer_reference]", &purchase.reference)
                .await
                .map_err(|_| anyhow::anyhow!("Order reconciliation failed"))?;
            let rows = rows["data"].as_array().context("Invalid order list")?;
            ensure!(
                rows.len() == 1,
                "Previous order outcome remains uncertain; do not submit another purchase"
            );
            rows[0].clone()
        } else {
            save(state, &purchase)?;
            let params = telnyx::numbers::CreateNumberOrderParams {
                connection_id: Some(connection.into()),
                customer_reference: Some(purchase.reference.clone()),
                ..telnyx::numbers::CreateNumberOrderParams::for_numbers([number])
            };
            let response: Value = t
                .client
                .post("/number_orders", &params)
                .max_retries(0)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Number purchase failed (status {:?}); order checkpoint preserved",
                        e.status().map(|s| s.as_u16())
                    )
                })?;
            response["data"].clone()
        };
        purchase.order_id = Some(
            response["id"]
                .as_str()
                .context("Missing order ID; creation is uncertain")?
                .to_owned(),
        );
        purchase.order = Some(response);
        save(state, &purchase)?;
    }
    let id = purchase.order_id.as_deref().context("Missing order ID")?;
    let response: Value = t
        .client
        .get(format!("/number_orders/{id}"))
        .await
        .map_err(|_| anyhow::anyhow!("Order status readback failed"))?;
    purchase.order = Some(response["data"].clone());
    save(state, &purchase)?;
    Ok(())
}
