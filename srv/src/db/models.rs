use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::db::schema::{
    api_call_logs, orders, payment_attempts, payment_events, product_info, products,
};

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
    pub product_id: Option<Uuid>,
    pub product_info_id: Uuid,
    pub product_name_snapshot: String,
    pub amount_cents: i64,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: String,
    pub contact: String,
    pub order_password_hash: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = orders)]
pub struct NewOrder<'a> {
    pub id: Uuid,
    pub product_id: Option<Uuid>,
    pub product_info_id: Uuid,
    pub product_name_snapshot: &'a str,
    pub amount_cents: i64,
    pub currency: &'a str,
    pub expires_at: DateTime<Utc>,
    pub status: &'a str,
    pub contact: &'a str,
    pub order_password_hash: &'a str,
}

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = payment_attempts)]
pub struct PaymentAttempt {
    pub id: Uuid,
    pub order_id: Uuid,
    pub provider: String,
    pub channel: String,
    pub merchant_trade_no: String,
    pub provider_transaction_id: Option<String>,
    pub state: String,
    pub code_url: Option<String>,
    pub amount_cents: i64,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = payment_attempts)]
pub struct NewPaymentAttempt<'a> {
    pub id: Uuid,
    pub order_id: Uuid,
    pub provider: &'a str,
    pub channel: &'a str,
    pub merchant_trade_no: &'a str,
    pub state: &'a str,
    pub amount_cents: i64,
    pub currency: &'a str,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = payment_events)]
pub struct NewPaymentEvent<'a> {
    pub id: Uuid,
    pub provider: &'a str,
    pub provider_event_id: &'a str,
    pub payment_attempt_id: Uuid,
    pub event_type: &'a str,
    pub request_body: &'a str,
    pub success: bool,
    pub error_message: Option<&'a str>,
}
