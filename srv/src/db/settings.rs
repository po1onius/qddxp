use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::{
    db::{models::SiteSettings, schema::site_settings},
    domain::OrderAllocationMode,
    error::AppError,
};

pub async fn load_order_allocation_mode(
    conn: &mut AsyncPgConnection,
) -> Result<OrderAllocationMode, AppError> {
    let mode = site_settings::table
        .select(site_settings::order_allocation_mode)
        .first::<String>(conn)
        .await
        .optional()?
        .unwrap_or_else(|| OrderAllocationMode::ReserveOnCreate.as_ref().to_string());

    mode.parse::<OrderAllocationMode>().map_err(|_| {
        tracing::error!(mode, "invalid order allocation mode in site settings");
        AppError::BadRequest("invalid order allocation mode".to_string())
    })
}

pub async fn save_order_allocation_mode(
    conn: &mut AsyncPgConnection,
    mode: OrderAllocationMode,
) -> Result<SiteSettings, AppError> {
    let now = Utc::now();

    diesel::insert_into(site_settings::table)
        .values((
            site_settings::id.eq(true),
            site_settings::order_allocation_mode.eq(mode.as_ref()),
            site_settings::updated_at.eq(now),
        ))
        .on_conflict(site_settings::id)
        .do_update()
        .set((
            site_settings::order_allocation_mode.eq(mode.as_ref()),
            site_settings::updated_at.eq(now),
        ))
        .get_result::<SiteSettings>(conn)
        .await
        .map_err(Into::into)
}
