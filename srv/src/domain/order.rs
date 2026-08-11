use strum::{AsRefStr, EnumString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum OrderStatus {
    Pending,
    Paid,
    Expired,
    Cancelled,
}
