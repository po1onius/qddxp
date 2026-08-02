use strum::{AsRefStr, EnumString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum PaymentType {
    Alipay,
    Wxpay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr)]
pub enum EpaySignType {
    #[strum(serialize = "MD5")]
    Md5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum ApiName {
    NotifyUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr)]
pub enum HttpMethod {
    #[strum(serialize = "GET")]
    Get,
    #[strum(serialize = "POST")]
    Post,
}
