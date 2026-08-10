use std::collections::BTreeMap;

use axum::{
    extract::{
        Form, Query, State,
        rejection::{FormRejection, QueryRejection},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use diesel_async::RunQueryDsl;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AppState,
    config::AppConfig,
    db::{models::NewApiCallLog, schema::api_call_logs},
    domain::{ApiName, EpaySignType, HttpMethod, PaymentProvider},
    error::AppError,
    payments::service::{PaymentConfirmation, confirm_payment},
};

const EPAY_TRADE_SUCCESS: &str = "TRADE_SUCCESS";
const EPAY_NOTIFY_SUCCESS: &str = "success";
const EPAY_NOTIFY_FAIL: &str = "fail";

pub fn build_payment_url(
    config: &AppConfig,
    order_id: Uuid,
    epay_trade_no: &str,
    product_name: &str,
    price_cents: i64,
    payment_type: &str,
) -> Option<String> {
    let epay = config.epay.as_ref()?;
    tracing::info!(
        %order_id,
        %epay_trade_no,
        payment_type,
        price_cents,
        "building epay payment url"
    );
    let notify_url = format!(
        "{}/api/payments/epay/notify",
        trim_end_slash(&config.public_base_url)
    );
    let money = format_money(price_cents);
    let order_param = order_id.to_string();

    // 参数表只在当前函数内参与签名和编码，因此可以直接借用配置与请求参数。
    // 仅为动态生成的通知地址、金额和订单参数分配字符串，避免复制商户号、返回地址等已有数据。
    let mut params = BTreeMap::from([
        ("pid", epay.pid.as_str()),
        ("type", payment_type),
        ("out_trade_no", epay_trade_no),
        ("notify_url", notify_url.as_str()),
        ("return_url", config.web_return_url.as_str()),
        ("name", product_name),
        ("money", money.as_str()),
        ("param", order_param.as_str()),
    ]);

    let sign = sign_params(&params, &epay.key);
    params.insert("sign", sign.as_str());
    params.insert("sign_type", EpaySignType::Md5.as_ref());

    let submit_url = build_submit_url(&epay.gateway);
    let separator = if submit_url.contains('?') { '&' } else { '?' };
    tracing::debug!(
        %order_id,
        %epay_trade_no,
        submit_url = %submit_url,
        "epay payment url parameters signed"
    );
    Some(format!(
        "{}{}{}",
        submit_url,
        separator,
        encode_query(&params)
    ))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EpayNotifyRequest {
    pub pid: String,
    pub name: String,
    pub money: String,
    pub out_trade_no: String,
    pub trade_no: String,
    pub param: Option<String>,
    pub trade_status: String,
    #[serde(rename = "type")]
    pub payment_type: String,
    pub sign: String,
    pub sign_type: String,
}

impl EpayNotifyRequest {
    fn signed_params(&self) -> BTreeMap<&str, &str> {
        let mut params = BTreeMap::from([
            ("money", self.money.as_str()),
            ("name", self.name.as_str()),
            ("out_trade_no", self.out_trade_no.as_str()),
            ("pid", self.pid.as_str()),
            ("sign", self.sign.as_str()),
            ("sign_type", self.sign_type.as_str()),
            ("trade_no", self.trade_no.as_str()),
            ("trade_status", self.trade_status.as_str()),
            ("type", self.payment_type.as_str()),
        ]);
        if let Some(param) = self.param.as_ref() {
            params.insert("param", param.as_str());
        }
        params
    }
}

pub async fn notify_query(
    State(state): State<AppState>,
    query: Result<Query<EpayNotifyRequest>, QueryRejection>,
) -> Response {
    match query {
        Ok(Query(notify)) => notify_response(state, HttpMethod::Get, notify).await,
        Err(error) => notify_parse_error_response(state, HttpMethod::Get, error.to_string()).await,
    }
}

pub async fn notify_form(
    State(state): State<AppState>,
    form: Result<Form<EpayNotifyRequest>, FormRejection>,
) -> Response {
    match form {
        Ok(Form(notify)) => notify_response(state, HttpMethod::Post, notify).await,
        Err(error) => notify_parse_error_response(state, HttpMethod::Post, error.to_string()).await,
    }
}

async fn notify_response(
    state: AppState,
    http_method: HttpMethod,
    notify: EpayNotifyRequest,
) -> Response {
    tracing::info!(
        http_method = http_method.as_ref(),
        out_trade_no = %notify.out_trade_no,
        trade_no = %notify.trade_no,
        trade_status = %notify.trade_status,
        payment_type = %notify.payment_type,
        amount = %notify.money,
        has_param = notify.param.as_ref().is_some_and(|value| !value.trim().is_empty()),
        "epay notify received"
    );
    let request_params = serde_json::to_value(&notify).unwrap_or_else(|_| json!({}));
    let validation_result = validate_notify(&state, &notify);
    let (success, response_body, error_message) = match validation_result {
        Ok(()) => {
            if notify.trade_status == EPAY_TRADE_SUCCESS {
                match apply_notify(&state, notify).await {
                    Ok(()) => (true, EPAY_NOTIFY_SUCCESS, None),
                    Err(error) => {
                        tracing::error!(
                            error = ?error,
                            "trusted epay notify business apply failed; gateway retry requested"
                        );
                        (
                            false,
                            EPAY_NOTIFY_FAIL,
                            Some(format!("business apply failed: {error}")),
                        )
                    }
                }
            } else {
                tracing::info!(
                    out_trade_no = %notify.out_trade_no,
                    trade_no = %notify.trade_no,
                    trade_status = %notify.trade_status,
                    "trusted epay notify accepted but trade is not successful; business apply skipped"
                );
                (true, EPAY_NOTIFY_SUCCESS, None)
            }
        }
        Err(error) => {
            tracing::warn!(error = ?error, "epay notify rejected before business apply");
            (false, EPAY_NOTIFY_FAIL, Some(error.to_string()))
        }
    };
    let has_error_message = error_message.is_some();

    if let Err(error) = record_notify_call(
        &state,
        http_method,
        request_params,
        success,
        response_body,
        error_message,
    )
    .await
    {
        tracing::warn!(error = ?error, "failed to record epay notify api log");
    }

    if success {
        tracing::info!(
            http_method = http_method.as_ref(),
            business_error = has_error_message,
            "epay notify accepted"
        );
    } else {
        tracing::warn!(http_method = http_method.as_ref(), "epay notify rejected");
    }

    (StatusCode::OK, response_body).into_response()
}

async fn notify_parse_error_response(
    state: AppState,
    http_method: HttpMethod,
    error_message: String,
) -> Response {
    if let Err(error) = record_notify_call(
        &state,
        http_method,
        json!({ "parse_error": error_message }),
        false,
        EPAY_NOTIFY_FAIL,
        Some("invalid notify request".to_string()),
    )
    .await
    {
        tracing::warn!(error = ?error, "failed to record epay notify api log");
    }

    tracing::warn!(error = %error_message, "epay notify parse failed");
    (StatusCode::OK, EPAY_NOTIFY_FAIL).into_response()
}

async fn record_notify_call(
    state: &AppState,
    http_method: HttpMethod,
    request_params: Value,
    success: bool,
    response_body: &str,
    error_message: Option<String>,
) -> Result<(), AppError> {
    let mut conn = state.pool.get().await?;
    diesel::insert_into(api_call_logs::table)
        .values(&NewApiCallLog {
            id: Uuid::new_v4(),
            api_name: ApiName::EpayNotify.as_ref(),
            http_method: http_method.as_ref(),
            path: "/api/payments/epay/notify",
            request_params: &request_params,
            response_status: i32::from(StatusCode::OK.as_u16()),
            response_body,
            success,
            error_message: error_message.as_deref(),
        })
        .execute(&mut conn)
        .await?;
    tracing::debug!(
        http_method = http_method.as_ref(),
        success,
        response_body,
        "epay notify api call log recorded"
    );

    Ok(())
}

fn validate_notify(state: &AppState, notify: &EpayNotifyRequest) -> Result<(), AppError> {
    tracing::debug!(
        out_trade_no = %notify.out_trade_no,
        trade_no = %notify.trade_no,
        "validating epay notify"
    );
    let epay = state.config.epay.as_ref().ok_or_else(|| {
        tracing::warn!("epay notify rejected: epay is not configured");
        AppError::BadRequest("epay is not configured".to_string())
    })?;

    if !notify
        .sign_type
        .eq_ignore_ascii_case(EpaySignType::Md5.as_ref())
    {
        tracing::warn!(
            sign_type = %notify.sign_type,
            "epay notify rejected: invalid sign type"
        );
        return Err(AppError::BadRequest("invalid sign_type".to_string()));
    }

    let expected_sign = sign_params(&notify.signed_params(), &epay.key);
    if !notify.sign.eq_ignore_ascii_case(&expected_sign) {
        tracing::warn!(
            out_trade_no = %notify.out_trade_no,
            trade_no = %notify.trade_no,
            "epay notify rejected: invalid signature"
        );
        return Err(AppError::BadRequest("invalid sign".to_string()));
    }

    if notify.pid != epay.pid {
        tracing::warn!(
            out_trade_no = %notify.out_trade_no,
            trade_no = %notify.trade_no,
            "epay notify rejected: invalid pid"
        );
        return Err(AppError::BadRequest("invalid pid".to_string()));
    }

    Ok(())
}

async fn apply_notify(state: &AppState, notify: EpayNotifyRequest) -> Result<(), AppError> {
    let merchant_trade_no = non_empty(&notify.out_trade_no, "missing out_trade_no")?;
    let expected_order_id = notify
        .param
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            Uuid::parse_str(value.trim())
                .map_err(|_| AppError::BadRequest("invalid param order id".to_string()))
        })
        .transpose()?;
    let paid_cents = money_to_cents(&notify.money)
        .ok_or_else(|| AppError::BadRequest("invalid money".to_string()))?;
    let provider_event_id = format!("notify:{}", notify.trade_no);
    let request_body = serde_json::to_string(&notify)
        .map_err(|error| AppError::BadRequest(format!("invalid epay notify: {error}")))?;

    confirm_payment(
        &state.pool,
        PaymentConfirmation {
            provider: PaymentProvider::Epay.as_ref(),
            provider_event_id: &provider_event_id,
            event_type: EPAY_TRADE_SUCCESS,
            merchant_trade_no,
            provider_transaction_id: &notify.trade_no,
            expected_order_id,
            amount_cents: paid_cents,
            currency: "CNY",
            paid_at: Utc::now(),
            request_body: &request_body,
        },
    )
    .await?;
    Ok(())
}

fn sign_params<K, V>(params: &BTreeMap<K, V>, key: &str) -> String
where
    K: AsRef<str> + Ord,
    V: AsRef<str>,
{
    let mut payload = String::new();
    for (param_key, value) in params {
        let param_key = param_key.as_ref();
        let value = value.as_ref();
        if param_key == "sign" || param_key == "sign_type" || value.is_empty() {
            continue;
        }

        if !payload.is_empty() {
            payload.push('&');
        }
        payload.push_str(param_key);
        payload.push('=');
        payload.push_str(value);
    }
    payload.push_str(key);

    format!("{:x}", md5::compute(payload))
}

fn non_empty<'a>(value: &'a str, message: &str) -> Result<&'a str, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(message.to_string()));
    }
    Ok(trimmed)
}

fn encode_query<K, V>(params: &BTreeMap<K, V>) -> String
where
    K: AsRef<str> + Ord,
    V: AsRef<str>,
{
    let mut query = String::new();
    for (key, value) in params {
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str(&urlencoding::encode(key.as_ref()));
        query.push('=');
        query.push_str(&urlencoding::encode(value.as_ref()));
    }
    query
}

fn format_money(price_cents: i64) -> String {
    format!("{}.{:02}", price_cents / 100, price_cents % 100)
}

fn money_to_cents(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    let (whole, fraction) = trimmed.split_once('.').unwrap_or((trimmed, ""));
    if whole.is_empty() || fraction.len() > 2 {
        return None;
    }

    let whole = whole.parse::<i64>().ok()?;
    let fraction = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().ok()? * 10,
        2 => fraction.parse::<i64>().ok()?,
        _ => return None,
    };

    Some(whole * 100 + fraction)
}

fn trim_end_slash(value: &str) -> &str {
    value.trim_end_matches('/')
}

fn build_submit_url(gateway: &str) -> String {
    let gateway = trim_end_slash(gateway.trim());
    if gateway.ends_with("/submit.php") {
        gateway.to_string()
    } else {
        format!("{gateway}/submit.php")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::EpayConfig;

    #[test]
    fn payment_url_contains_complete_parameters_and_valid_signature() {
        let config = AppConfig {
            listen_addr: "127.0.0.1:3000".parse().expect("监听地址应当有效"),
            database_url: "postgres://example.invalid/test".to_string(),
            public_base_url: "https://shop.example.com/".to_string(),
            web_return_url: "https://shop.example.com/delivery".to_string(),
            web_dist_dir: PathBuf::from("web/dist"),
            admin_key: "test-admin-key".to_string(),
            order_password_pepper: "test-password-pepper".to_string(),
            payment_expire_minutes: 15,
            epay: Some(EpayConfig {
                gateway: "https://pay.example.com/".to_string(),
                pid: "merchant-id".to_string(),
                key: "merchant-secret".to_string(),
            }),
            wechatpay: None,
        };
        let order_id =
            Uuid::parse_str("019c2ddf-78fb-7fe0-8ed8-22577c255c83").expect("测试订单 ID 应当有效");

        let payment_url =
            build_payment_url(&config, order_id, "trade-001", "测试商品", 1_099, "alipay")
                .expect("支付配置完整时应当生成支付地址");
        let (submit_url, query) = payment_url
            .split_once('?')
            .expect("支付地址应当包含查询参数");
        let params = query
            .split('&')
            .map(|pair| {
                let (key, value) = pair.split_once('=').expect("查询参数应当包含等号");
                let key = urlencoding::decode(key)
                    .expect("参数名应当可以解码")
                    .into_owned();
                let value = urlencoding::decode(value)
                    .expect("参数值应当可以解码")
                    .into_owned();
                (key, value)
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(submit_url, "https://pay.example.com/submit.php");
        assert_eq!(params.get("pid").map(String::as_str), Some("merchant-id"));
        assert_eq!(params.get("money").map(String::as_str), Some("10.99"));
        assert_eq!(params.get("name").map(String::as_str), Some("测试商品"));
        assert_eq!(
            params.get("notify_url").map(String::as_str),
            Some("https://shop.example.com/api/payments/epay/notify")
        );
        assert_eq!(
            params.get("sign").map(String::as_str),
            Some(sign_params(&params, "merchant-secret").as_str())
        );
    }
}
