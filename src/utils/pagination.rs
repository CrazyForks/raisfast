//! 分页参数提取与校验模块
//!
//! 从 Axum 查询字符串中提取 `page` 和 `page_size` 参数，
//! 并自动进行边界校验，确保分页值在安全范围内。

use axum::extract::Query;
use serde::Deserialize;

use crate::errors::response::{ApiResponse, PaginatedData};

/// 分页查询参数。
///
/// - `page`：当前页码，默认为 1。
/// - `page_size`：每页条数，默认为 20，最大不超过 [`MAX_PAGE_SIZE`](PaginationParams::MAX_PAGE_SIZE)。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    20
}

impl PaginationParams {
    /// 允许的最大每页条数。
    pub const MAX_PAGE_SIZE: i64 = 100;

    /// 将分页结果包装为标准 API 响应。
    ///
    /// 将 `items` 列表和 `total` 总数与当前分页参数组合为 [`PaginatedData`]，
    /// 再包装为 [`ApiResponse::success`]，供 handler 直接返回。
    pub fn paginate<T: serde::Serialize>(
        self,
        items: Vec<T>,
        total: i64,
    ) -> ApiResponse<PaginatedData<T>> {
        ApiResponse::success(PaginatedData {
            items,
            total,
            page: self.page,
            page_size: self.page_size,
        })
    }

    /// 从可选的 page / page_size 构建分页参数。
    ///
    /// 自动执行校验，适合 handler 中从 `Option<i64>` 查询参数构建。
    pub fn from_options(page: Option<i64>, page_size: Option<i64>) -> Self {
        let mut params = Self::default();
        if let Some(p) = page {
            params.page = p.max(1);
        }
        if let Some(ps) = page_size {
            params.page_size = ps.clamp(1, Self::MAX_PAGE_SIZE);
        }
        params.sanitize();
        params
    }

    /// 对内存中的 Vec 进行分页切片。
    pub fn paginate_in_memory<T>(self, all: Vec<T>) -> ApiResponse<PaginatedData<T>>
    where
        T: serde::Serialize,
    {
        let total = all.len() as i64;
        let offset = self.offset() as usize;
        let items: Vec<_> = all
            .into_iter()
            .skip(offset)
            .take(self.page_size as usize)
            .collect();
        self.paginate(items, total)
    }

    /// 计算 SQL `OFFSET` 值。
    ///
    /// 公式：`(page - 1) * page_size`。
    #[must_use]
    pub fn offset(&self) -> i64 {
        (self.page - 1).saturating_mul(self.page_size)
    }

    /// 校验并修正分页参数。
    ///
    /// - 将 `page` 钳位为 ≥ 1。
    /// - 将 `page_size` 钳位到 `[1, MAX_PAGE_SIZE]` 范围内。
    pub fn sanitize(&mut self) {
        self.page = self.page.max(1);
        self.page_size = self.page_size.clamp(1, Self::MAX_PAGE_SIZE);
    }
}

/// 从 Axum `Query` 提取器中解析分页参数，并自动执行校验。
impl From<Query<PaginationParams>> for PaginationParams {
    fn from(query: Query<PaginationParams>) -> Self {
        let mut params = query.0;
        params.sanitize();
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_page1() {
        let p = PaginationParams {
            page: 1,
            page_size: 20,
        };
        assert_eq!(p.offset(), 0);
    }

    #[test]
    fn offset_page3() {
        let p = PaginationParams {
            page: 3,
            page_size: 10,
        };
        assert_eq!(p.offset(), 20);
    }

    #[test]
    fn sanitize_clamps_page_to_one() {
        let mut p = PaginationParams {
            page: -5,
            page_size: 20,
        };
        p.sanitize();
        assert_eq!(p.page, 1);
    }

    #[test]
    fn sanitize_clamps_page_size_to_max() {
        let mut p = PaginationParams {
            page: 1,
            page_size: 999,
        };
        p.sanitize();
        assert_eq!(p.page_size, PaginationParams::MAX_PAGE_SIZE);
    }

    #[test]
    fn sanitize_clamps_page_size_to_one() {
        let mut p = PaginationParams {
            page: 1,
            page_size: 0,
        };
        p.sanitize();
        assert_eq!(p.page_size, 1);
    }
}
