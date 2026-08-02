use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const DEFAULT_OFFSET_PAGE_SIZE: i64 = 20;
pub const MAX_PAGE_SIZE: i64 = 100;

#[derive(Debug, Deserialize, Default)]
pub struct OffsetPagination {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct OffsetPageResponse<T> {
    pub items: Vec<T>,
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
}

pub fn normalize_offset_page(input: &OffsetPagination) -> Result<(i64, i64, i64), AppError> {
    let page = input.page.unwrap_or(1);
    if page < 1 {
        return Err(AppError::BadRequest(
            "page must be greater than 0".to_string(),
        ));
    }

    let page_size = normalize_limit(input.page_size, DEFAULT_OFFSET_PAGE_SIZE)?;
    Ok((page, page_size, (page - 1) * page_size))
}

fn normalize_limit(value: Option<i64>, default: i64) -> Result<i64, AppError> {
    let limit = value.unwrap_or(default);
    if limit < 1 {
        return Err(AppError::BadRequest(
            "page_size must be greater than 0".to_string(),
        ));
    }

    Ok(limit.min(MAX_PAGE_SIZE))
}
