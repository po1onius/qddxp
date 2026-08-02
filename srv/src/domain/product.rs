use strum::{AsRefStr, EnumString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum ProductStatus {
    Available,
    Reserved,
    Delivered,
    Disabled,
}
