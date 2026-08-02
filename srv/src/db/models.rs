use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::db::schema::{api_call_logs, orders, product_info, products, site_settings};

#[derive(Debug, Clone, Queryable, Identifiable, Serialize)]
#[diesel(table_name = api_call_logs)]
pub struct ApiCallLog {
    pub id: Uuid,
    pub api_name: String,
    pub http_method: String,
    pub path: String,
    pub request_params: Value,
    pub response_status: i32,
    pub response_body: String,
    pub success: bool,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = api_call_logs)]
pub struct NewApiCallLog<'a> {
    pub id: Uuid,
    pub api_name: &'a str,
    pub http_method: &'a str,
    pub path: &'a str,
    pub request_params: &'a Value,
    pub response_status: i32,
    pub response_body: &'a str,
    pub success: bool,
    pub error_message: Option<&'a str>,
}

#[derive(Debug, Clone, Queryable, Identifiable, Serialize)]
#[diesel(table_name = product_info)]
pub struct ProductInfo {
    pub id: Uuid,
    pub image_base64: Option<String>,
    pub name: String,
    pub details: String,
    pub price_cents: i64,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = product_info)]
pub struct NewProductInfo<'a> {
    pub id: Uuid,
    pub image_base64: Option<&'a str>,
    pub name: &'a str,
    pub details: &'a str,
    pub price_cents: i64,
    pub active: bool,
}

#[derive(Debug, Clone, Queryable, Identifiable, Serialize)]
#[diesel(table_name = products)]
pub struct Product {
    pub id: Uuid,
    pub product_info_id: Uuid,
    pub content: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = products)]
pub struct NewProduct<'a> {
    pub id: Uuid,
    pub product_info_id: Uuid,
    pub content: &'a str,
    pub status: &'a str,
}

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = orders)]
pub struct Order {
    pub id: Uuid,
    pub epay_trade_no: String,
    pub product_id: Option<Uuid>,
    pub product_info_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    pub status: String,
    pub contact: String,
    pub order_password_hash: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = orders)]
pub struct NewOrder<'a> {
    pub id: Uuid,
    pub epay_trade_no: &'a str,
    pub product_id: Option<Uuid>,
    pub product_info_id: Uuid,
    pub status: &'a str,
    pub contact: &'a str,
    pub order_password_hash: &'a str,
}

#[derive(Debug, Clone, Queryable, Identifiable, Serialize)]
#[diesel(table_name = site_settings)]
pub struct SiteSettings {
    pub id: bool,
    pub order_allocation_mode: String,
    pub updated_at: DateTime<Utc>,
}
