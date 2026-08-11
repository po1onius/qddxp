pub mod expiration;
pub mod service;
pub mod wechatpay;

/// ePay 协议没有可由商户设置的支付结束时间，本地最多预占库存三分钟。
/// 超过该时间才到达的可信支付通知只记账，不重新分配库存或发货。
pub const EPAY_RESERVATION_MINUTES: i64 = 3;
