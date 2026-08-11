use strum::{AsRefStr, EnumString};

/// 支付提供方。`Epay` 与微信支付官方直连是两套完全独立的协议，禁止混用配置和回调。
#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum PaymentProvider {
    Epay,
    Wechatpay,
}

/// 支付渠道。渠道必须与提供方组合校验，例如 `epay/wxpay` 与
/// `wechatpay/native` 虽然最终都使用微信客户端付款，但协议层完全不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum PaymentChannel {
    Alipay,
    Wxpay,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum PaymentAttemptState {
    Created,
    /// 已完成当前支付方要求的准备步骤，可以向用户展示跳转地址或付款二维码。
    Ready,
    Succeeded,
    Failed,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr)]
pub enum EpaySignType {
    #[strum(serialize = "MD5")]
    Md5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum ApiName {
    EpayNotify,
    WechatpayNotify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr)]
pub enum HttpMethod {
    #[strum(serialize = "GET")]
    Get,
    #[strum(serialize = "POST")]
    Post,
}

/// 校验提供方与渠道组合，避免把易支付的 `wxpay` 参数误认为微信官方协议。
pub fn validate_payment_method(provider: PaymentProvider, channel: PaymentChannel) -> bool {
    matches!(
        (provider, channel),
        (PaymentProvider::Epay, PaymentChannel::Alipay)
            | (PaymentProvider::Epay, PaymentChannel::Wxpay)
            | (PaymentProvider::Wechatpay, PaymentChannel::Native)
    )
}
