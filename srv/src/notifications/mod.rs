use std::{sync::Arc, time::Duration as StdDuration};

use chrono::{DateTime, Utc};
use teloxide_core::{
    Bot,
    requests::Requester,
    types::{ChatId, Recipient},
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    config::TelegramConfig,
    db::models::{Order, PaymentAttempt},
};

/// Telegram 只是辅助通知渠道，不参与订单和支付结果的可靠交付。
/// 每个业务事件只发送一次，超过该时间即放弃并记录日志，不重试、不落库。
const SEND_TIMEOUT_SECONDS: u64 = 15;

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
        // 因此网络、解析和 IO 错误只保留安全类别，避免日志泄露生产密钥。
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

/// Telegram 客户端在应用启动时完成配置校验；业务路径只负责提交一次异步发送任务。
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

/// 订单事务提交后异步通知一次。未配置 Telegram 时直接跳过，不影响下单结果。
pub fn notify_order_created(
    notifier: Option<Arc<TelegramNotifier>>,
    order: &Order,
    attempt: &PaymentAttempt,
) {
    let Some(notifier) = notifier else {
        return;
    };
    let text = render_order_created(order, attempt);
    spawn_notification(notifier, "order.created", order.id, Some(attempt.id), text);
}

/// 支付确认事务提交后异步通知一次。重复回调不会调用本函数，因此不会重复提交发送任务。
pub fn notify_payment_confirmed(
    notifier: Option<Arc<TelegramNotifier>>,
    order: &Order,
    attempt: &PaymentAttempt,
    provider_transaction_id: &str,
    paid_at: DateTime<Utc>,
    delivered: bool,
) {
    let Some(notifier) = notifier else {
        return;
    };
    let text =
        render_payment_confirmed(order, attempt, provider_transaction_id, paid_at, delivered);
    spawn_notification(
        notifier,
        "payment.confirmed",
        order.id,
        Some(attempt.id),
        text,
    );
}

/// 发送任务与 HTTP 请求和数据库事务完全解耦。失败只记录一次结构化日志，明确不重试。
fn spawn_notification(
    notifier: Arc<TelegramNotifier>,
    event_type: &'static str,
    order_id: Uuid,
    payment_attempt_id: Option<Uuid>,
    text: String,
) {
    tokio::spawn(async move {
        tracing::debug!(
            event_type,
            %order_id,
            ?payment_attempt_id,
            "sending Telegram notification once"
        );
        match tokio::time::timeout(
            StdDuration::from_secs(SEND_TIMEOUT_SECONDS),
            notifier.send(text),
        )
        .await
        {
            Ok(Ok(message_id)) => {
                tracing::info!(
                    event_type,
                    %order_id,
                    ?payment_attempt_id,
                    telegram_message_id = message_id,
                    "Telegram notification sent"
                );
            }
            Ok(Err(error)) => {
                tracing::error!(
                    event_type,
                    %order_id,
                    ?payment_attempt_id,
                    error = %error,
                    "Telegram notification failed; no retry will be attempted"
                );
            }
            Err(_) => {
                tracing::error!(
                    event_type,
                    %order_id,
                    ?payment_attempt_id,
                    timeout_seconds = SEND_TIMEOUT_SECONDS,
                    "Telegram notification timed out; no retry will be attempted"
                );
            }
        }
    });
}

fn render_order_created(order: &Order, attempt: &PaymentAttempt) -> String {
    format!(
        "🛒 新订单\n\n订单：{}\n商品：{}\n金额：{}\n支付：{}\n联系：{}\n状态：待付款\n下单时间：{}\n过期时间：{}",
        order.id,
        order.product_name_snapshot,
        format_money(order.amount_cents, &order.currency),
        payment_label(&attempt.provider, &attempt.channel),
        mask_contact(&order.contact),
        format_time(order.created_at),
        format_time(order.expires_at),
    )
}

fn render_payment_confirmed(
    order: &Order,
    attempt: &PaymentAttempt,
    provider_transaction_id: &str,
    paid_at: DateTime<Utc>,
    delivered: bool,
) -> String {
    let (heading, result) = if delivered {
        ("✅ 支付成功", "已自动交付")
    } else {
        ("🚨 超时后到账", "已收款但未交付，请立即人工处理")
    };
    format!(
        "{heading}\n\n订单：{}\n商品：{}\n金额：{}\n支付：{}\n交易号：{}\n支付时间：{}\n处理结果：{result}",
        order.id,
        order.product_name_snapshot,
        format_money(order.amount_cents, &order.currency),
        payment_label(&attempt.provider, &attempt.channel),
        provider_transaction_id,
        format_time(paid_at),
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
