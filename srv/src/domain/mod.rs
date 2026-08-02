pub mod order;
pub mod payment;
pub mod product;

pub use order::{OrderAllocationMode, OrderStatus};
pub use payment::{ApiName, EpaySignType, HttpMethod, PaymentType};
pub use product::ProductStatus;

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn enum_strings_match_database_values() {
        assert_eq!(ProductStatus::Available.as_ref(), "available");
        assert_eq!(ProductStatus::Reserved.as_ref(), "reserved");
        assert_eq!(ProductStatus::Delivered.as_ref(), "delivered");
        assert_eq!(ProductStatus::Disabled.as_ref(), "disabled");
        assert_eq!(OrderStatus::Pending.as_ref(), "pending");
        assert_eq!(OrderStatus::Paid.as_ref(), "paid");
        assert_eq!(OrderStatus::Preorder.as_ref(), "preorder");
        assert_eq!(
            OrderAllocationMode::ReserveOnCreate.as_ref(),
            "reserve_on_create"
        );
        assert_eq!(
            OrderAllocationMode::AllocateOnPay.as_ref(),
            "allocate_on_pay"
        );
        assert_eq!(PaymentType::Alipay.as_ref(), "alipay");
        assert_eq!(PaymentType::Wxpay.as_ref(), "wxpay");
        assert_eq!(EpaySignType::Md5.as_ref(), "MD5");
        assert_eq!(ApiName::NotifyUrl.as_ref(), "notify_url");
        assert_eq!(HttpMethod::Get.as_ref(), "GET");
        assert_eq!(HttpMethod::Post.as_ref(), "POST");
    }

    #[test]
    fn request_values_parse_into_enums() {
        assert_eq!(
            ProductStatus::from_str("available"),
            Ok(ProductStatus::Available)
        );
        assert_eq!(OrderStatus::from_str("pending"), Ok(OrderStatus::Pending));
        assert_eq!(OrderStatus::from_str("paid"), Ok(OrderStatus::Paid));
        assert_eq!(OrderStatus::from_str("preorder"), Ok(OrderStatus::Preorder));
        assert_eq!(
            OrderAllocationMode::from_str("allocate_on_pay"),
            Ok(OrderAllocationMode::AllocateOnPay)
        );
        assert!(OrderStatus::from_str("delivered").is_err());
        assert!(OrderStatus::from_str("cancelled").is_err());
        assert_eq!(PaymentType::from_str("wxpay"), Ok(PaymentType::Wxpay));
    }
}
