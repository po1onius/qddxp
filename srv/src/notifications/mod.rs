use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use teloxide_core::{
    Bot,
    requests::Requester,
    types::{ChatId, Recipient},
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    AppState,
    config::TelegramConfig,
    db::{
        models::{NewNotificationOutbox, NotificationOutbox, Order, PaymentAttempt},
        schema::notification_outbox,
    },
    error::AppError,
};

const EVENT_ORDER_CREATED: &str = "order.created";
const EVENT_PAYMENT_CONFIRMED: &str = "payment.confirmed";
const STATUS_PENDING: &str = "pending";
const STATUS_PROCESSING: &str = "processing";
const STATUS_SENT: &str = "sent";
const POLL_INTERVAL_SECONDS: u64 = 2;
const LEASE_SECONDS: i64 = 30;
const MAX_BATCH_SIZE: usize = 20;
const MAX_RETRY_DELAY_SECONDS: i64 = 3_600;
const MAX_STORED_ERROR_CHARS: usize = 1_000;

#[derive(Debug, Error)]
pub enum TelegramNotifierError {
    #[error("invalid Telegram notify chat id: use a numeric chat id or @channelusername")]
    InvalidChatId,
    #[error("{0}")]
    Request(String),
}

impl From<teloxide_core::RequestError> for TelegramNotifierError {
    fn from(error: teloxide_core::RequestError) -> Self {
        use teloxide_core::RequestError;

        // reqwest 的网络错误文本可能包含完整请求 URL，而 Telegram URL 中嵌入 Bot Token。
        // 因此网络、解析和 IO 错误只保留安全类别；API 业务错误来自响应 description，
        // 不包含请求 URL，可以用于判断 chat not found、权限不足等配置问题。
        let message = match error {
            RequestError::Api(error) => format!("Telegram API error: {error}"),
            RequestError::MigrateToChatId(chat_id) => {
                format!("Telegram chat migrated to {chat_id}")
            }
            RequestError::RetryAfter(seconds) => {
                format!("Telegram rate limited the bot; retry after {seconds}")
            }
            RequestError::Network(_) => "Telegram network request failed".to_string(),
            RequestError::InvalidJson { .. } => {
                "Telegram returned an invalid JSON response".to_string()
            }
            RequestError::Io(_) => "Telegram request IO failed".to_string(),
        };
        Self::Request(message)
    }
}

/// Telegram 只负责传输已经持久化的通知，不持有数据库连接或业务状态。
/// Recipient 在启动时解析一次，避免格式错误的 chat id 让每条 Outbox 都永久重试。
pub struct TelegramNotifier {
    bot: Bot,
    recipient: Recipient,
}

impl TelegramNotifier {
    pub fn from_config(config: &TelegramConfig) -> Result<Self, TelegramNotifierError> {
        let recipient = parse_recipient(&config.chat_id)?;
        Ok(Self {
            bot: Bot::new(&config.bot_token),
            recipient,
        })
    }

    async fn send(&self, text: String) -> Result<i64, TelegramNotifierError> {
        let message = self.bot.send_message(self.recipient.clone(), text).await?;
        Ok(i64::from(message.id.0))
    }
}

fn parse_recipient(value: &str) -> Result<Recipient, TelegramNotifierError> {
    if let Ok(chat_id) = value.parse::<i64>() {
        return Ok(Recipient::Id(ChatId(chat_id)));
    }
    if value.starts_with('@') && value.len() > 1 {
        return Ok(Recipient::ChannelUsername(value.to_string()));
    }
    Err(TelegramNotifierError::InvalidChatId)
}

#[derive(Debug, Serialize, Deserialize)]
struct OrderCreatedPayload {
    order_id: Uuid,
    product_name: String,
    amount_cents: i64,
    currency: String,
    provider: String,
    channel: String,
    contact_masked: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PaymentConfirmedPayload {
    order_id: Uuid,
    payment_attempt_id: Uuid,
    product_name: String,
    amount_cents: i64,
    currency: String,
    provider: String,
    channel: String,
    provider_transaction_id: String,
    paid_at: DateTime<Utc>,
    delivered: bool,
}

/// 与创建订单事务一同写入通知事件。event_key 唯一约束提供业务级幂等保护。
pub async fn enqueue_order_created(
    conn: &mut diesel_async::AsyncPgConnection,
    order: &Order,
    attempt: &PaymentAttempt,
) -> Result<(), AppError> {
    let event_key = format!("{EVENT_ORDER_CREATED}:{}", order.id);
    let payload = serde_json::to_value(OrderCreatedPayload {
        order_id: order.id,
        product_name: order.product_name_snapshot.clone(),
        amount_cents: order.amount_cents,
        currency: order.currency.clone(),
        provider: attempt.provider.clone(),
        channel: attempt.channel.clone(),
        // Outbox 仅保存消息真正需要的脱敏值，避免数据库新增一份可恢复的联系方式副本。
        contact_masked: mask_contact(&order.contact),
        created_at: order.created_at,
        expires_at: order.expires_at,
    })
    .map_err(|error| {
        AppError::BadRequest(format!("cannot serialize order notification: {error}"))
    })?;
    insert_event(conn, &event_key, EVENT_ORDER_CREATED, &payload).await
}

/// 仅在首次确认付款的事务中调用。超时后到账也会产生事件，但 delivered=false，
/// Telegram 文案会提升为人工介入告警，避免已收款未交付长期无人处理。
pub async fn enqueue_payment_confirmed(
    conn: &mut diesel_async::AsyncPgConnection,
    order: &Order,
    attempt: &PaymentAttempt,
    provider_transaction_id: &str,
    paid_at: DateTime<Utc>,
    delivered: bool,
) -> Result<(), AppError> {
    let event_key = format!("{EVENT_PAYMENT_CONFIRMED}:{}", attempt.id);
    let payload = serde_json::to_value(PaymentConfirmedPayload {
        order_id: order.id,
        payment_attempt_id: attempt.id,
        product_name: order.product_name_snapshot.clone(),
        amount_cents: order.amount_cents,
        currency: order.currency.clone(),
        provider: attempt.provider.clone(),
        channel: attempt.channel.clone(),
        provider_transaction_id: provider_transaction_id.to_string(),
        paid_at,
        delivered,
    })
    .map_err(|error| {
        AppError::BadRequest(format!("cannot serialize payment notification: {error}"))
    })?;
    insert_event(conn, &event_key, EVENT_PAYMENT_CONFIRMED, &payload).await
}

async fn insert_event(
    conn: &mut diesel_async::AsyncPgConnection,
    event_key: &str,
    event_type: &str,
    payload: &Value,
) -> Result<(), AppError> {
    diesel::insert_into(notification_outbox::table)
        .values(&NewNotificationOutbox {
            id: Uuid::new_v4(),
            event_key,
            event_type,
            payload,
        })
        // 唯一事件已经存在时视为成功，使调用方在未来重构或补偿时仍然幂等。
        .on_conflict(notification_outbox::event_key)
        .do_nothing()
        .execute(conn)
        .await?;
    tracing::info!(event_key, event_type, "notification event enqueued");
    Ok(())
}

/// 循环领取持久化事件并发送。每条任务都使用租约，实例退出或发送过程中崩溃后，
/// 其他实例能够在 locked_until 到期后自动接管，不需要人工修改数据库状态。
pub async fn run(state: AppState) {
    tracing::info!(
        poll_interval_seconds = POLL_INTERVAL_SECONDS,
        lease_seconds = LEASE_SECONDS,
        max_batch_size = MAX_BATCH_SIZE,
        "Telegram notification worker started"
    );
    let mut interval = tokio::time::interval(StdDuration::from_secs(POLL_INTERVAL_SECONDS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;
        if let Err(error) = process_batch(&state).await {
            tracing::error!(error = ?error, "Telegram notification batch failed");
        }
    }
}

async fn process_batch(state: &AppState) -> Result<(), AppError> {
    let notifier = state
        .telegram
        .as_ref()
        .expect("notification worker starts only when Telegram is configured");
    for _ in 0..MAX_BATCH_SIZE {
        let Some(event) = claim_event(state).await? else {
            break;
        };
        let text = match render_message(&event) {
            Ok(text) => text,
            Err(error) => {
                tracing::error!(
                    notification_id = %event.id,
                    event_key = %event.event_key,
                    error = %error,
                    "notification payload is invalid"
                );
                mark_failed(state, &event, &error).await?;
                continue;
            }
        };

        match notifier.send(text).await {
            Ok(message_id) => mark_sent(state, &event, message_id).await?,
            Err(error) => {
                tracing::warn!(
                    notification_id = %event.id,
                    event_key = %event.event_key,
                    attempt_count = event.attempt_count,
                    error = %error,
                    "Telegram notification send failed; retry scheduled"
                );
                mark_failed(state, &event, &error.to_string()).await?;
            }
        }
    }
    Ok(())
}

async fn claim_event(state: &AppState) -> Result<Option<NotificationOutbox>, AppError> {
    let mut conn = state.pool.get().await?;
    conn.transaction::<_, AppError, _>(async move |conn| {
        let now = Utc::now();
        let event = notification_outbox::table
            .filter(notification_outbox::next_attempt_at.le(now))
            .filter(
                notification_outbox::status
                    .eq(STATUS_PENDING)
                    .or(notification_outbox::status
                        .eq(STATUS_PROCESSING)
                        .and(notification_outbox::locked_until.le(Some(now)))),
            )
            .order((
                notification_outbox::next_attempt_at.asc(),
                notification_outbox::created_at.asc(),
            ))
            .for_update()
            .skip_locked()
            .first::<NotificationOutbox>(conn)
            .await
            .optional()?;
        let Some(event) = event else {
            return Ok(None);
        };
        let claimed = diesel::update(notification_outbox::table.find(event.id))
            .set((
                notification_outbox::status.eq(STATUS_PROCESSING),
                notification_outbox::attempt_count.eq(event.attempt_count + 1),
                notification_outbox::locked_until.eq(Some(now + Duration::seconds(LEASE_SECONDS))),
                notification_outbox::updated_at.eq(now),
            ))
            .get_result::<NotificationOutbox>(conn)
            .await?;
        tracing::debug!(
            notification_id = %claimed.id,
            event_key = %claimed.event_key,
            attempt_count = claimed.attempt_count,
            "notification event claimed"
        );
        Ok(Some(claimed))
    })
    .await
}

async fn mark_sent(
    state: &AppState,
    event: &NotificationOutbox,
    message_id: i64,
) -> Result<(), AppError> {
    let now = Utc::now();
    let mut conn = state.pool.get().await?;
    diesel::update(notification_outbox::table.find(event.id))
        .set((
            notification_outbox::status.eq(STATUS_SENT),
            notification_outbox::locked_until.eq::<Option<DateTime<Utc>>>(None),
            notification_outbox::last_error.eq::<Option<String>>(None),
            notification_outbox::telegram_message_id.eq(Some(message_id)),
            notification_outbox::sent_at.eq(Some(now)),
            notification_outbox::updated_at.eq(now),
        ))
        .execute(&mut conn)
        .await?;
    tracing::info!(
        notification_id = %event.id,
        event_key = %event.event_key,
        telegram_message_id = message_id,
        attempt_count = event.attempt_count,
        "Telegram notification sent"
    );
    Ok(())
}

async fn mark_failed(
    state: &AppState,
    event: &NotificationOutbox,
    error: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    let retry_delay = retry_delay_seconds(event.attempt_count);
    let stored_error = truncate_chars(error, MAX_STORED_ERROR_CHARS);
    let mut conn = state.pool.get().await?;
    diesel::update(notification_outbox::table.find(event.id))
        .set((
            notification_outbox::status.eq(STATUS_PENDING),
            notification_outbox::next_attempt_at.eq(now + Duration::seconds(retry_delay)),
            notification_outbox::locked_until.eq::<Option<DateTime<Utc>>>(None),
            notification_outbox::last_error.eq(Some(stored_error)),
            notification_outbox::updated_at.eq(now),
        ))
        .execute(&mut conn)
        .await?;
    Ok(())
}

fn retry_delay_seconds(attempt_count: i32) -> i64 {
    let exponent = attempt_count.clamp(1, 11) as u32 - 1;
    (5_i64.saturating_mul(2_i64.saturating_pow(exponent))).min(MAX_RETRY_DELAY_SECONDS)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn render_message(event: &NotificationOutbox) -> Result<String, String> {
    match event.event_type.as_str() {
        EVENT_ORDER_CREATED => serde_json::from_value::<OrderCreatedPayload>(event.payload.clone())
            .map(|payload| render_order_created(&payload))
            .map_err(|error| error.to_string()),
        EVENT_PAYMENT_CONFIRMED => {
            serde_json::from_value::<PaymentConfirmedPayload>(event.payload.clone())
                .map(|payload| render_payment_confirmed(&payload))
                .map_err(|error| error.to_string())
        }
        other => Err(format!("unsupported notification event type: {other}")),
    }
}

fn render_order_created(payload: &OrderCreatedPayload) -> String {
    format!(
        "🛒 新订单\n\n订单：{}\n商品：{}\n金额：{}\n支付：{}\n联系：{}\n状态：待付款\n下单时间：{}\n过期时间：{}",
        payload.order_id,
        payload.product_name,
        format_money(payload.amount_cents, &payload.currency),
        payment_label(&payload.provider, &payload.channel),
        payload.contact_masked,
        format_time(payload.created_at),
        format_time(payload.expires_at),
    )
}

fn render_payment_confirmed(payload: &PaymentConfirmedPayload) -> String {
    let (heading, result) = if payload.delivered {
        ("✅ 支付成功", "已自动交付")
    } else {
        ("🚨 超时后到账", "已收款但未交付，请立即人工处理")
    };
    format!(
        "{heading}\n\n订单：{}\n商品：{}\n金额：{}\n支付：{}\n交易号：{}\n支付时间：{}\n处理结果：{result}",
        payload.order_id,
        payload.product_name,
        format_money(payload.amount_cents, &payload.currency),
        payment_label(&payload.provider, &payload.channel),
        payload.provider_transaction_id,
        format_time(payload.paid_at),
    )
}

fn format_money(amount_cents: i64, currency: &str) -> String {
    let sign = if amount_cents < 0 { "-" } else { "" };
    let absolute = amount_cents.unsigned_abs();
    let symbol = if currency == "CNY" { "¥" } else { currency };
    format!("{sign}{symbol}{}.{:02}", absolute / 100, absolute % 100)
}

fn format_time(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn payment_label(provider: &str, channel: &str) -> String {
    match (provider, channel) {
        ("epay", "alipay") => "支付宝（易支付）".to_string(),
        ("epay", "wxpay") => "微信（易支付）".to_string(),
        ("wechatpay", "native") => "微信支付（官方）".to_string(),
        _ => format!("{provider}/{channel}"),
    }
}

fn mask_contact(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    match chars.len() {
        0 => String::new(),
        1..=4 => "*".repeat(chars.len()),
        len => format!(
            "{}{}{}",
            chars[..2].iter().collect::<String>(),
            "*".repeat(len - 4),
            chars[len - 2..].iter().collect::<String>()
        ),
    }
}
