//! Pagination parameter extraction and validation module
//!
//! Extracts `page` and `page_size` parameters from Axum query strings,
//! and automatically performs boundary validation to ensure safe pagination values.

use axum::extract::Query;
use serde::Deserialize;

use crate::errors::response::{ApiResponse, PaginatedData};

/// Pagination query parameters.
///
/// - `page`: Current page number, defaults to 1.
/// - `page_size`: Items per page, defaults to 20, max is [`MAX_PAGE_SIZE`](PaginationParams::MAX_PAGE_SIZE).
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
    /// Maximum allowed items per page.
    pub const MAX_PAGE_SIZE: i64 = 100;

    /// Wraps paginated results as a standard API response.
    ///
    /// Combines the `items` list and `total` count with current pagination parameters
    /// into [`PaginatedData`], then wraps as [`ApiResponse::success`] for direct handler return.
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

    /// Builds pagination parameters from optional page / page_size.
    ///
    /// Automatically performs validation; suitable for constructing from `Option<i64>` query params in handlers.
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

    /// Paginates an in-memory Vec by slicing.
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

    /// Computes the SQL `OFFSET` value.
    ///
    /// Formula: `(page - 1) * page_size`.
    #[must_use]
    pub fn offset(&self) -> i64 {
        (self.page - 1).saturating_mul(self.page_size)
    }

    /// Validates and corrects pagination parameters.
    ///
    /// - Clamps `page` to >= 1.
    /// - Clamps `page_size` to the range `[1, MAX_PAGE_SIZE]`.
    pub fn sanitize(&mut self) {
        self.page = self.page.max(1);
        self.page_size = self.page_size.clamp(1, Self::MAX_PAGE_SIZE);
    }
}

/// Parses pagination parameters from an Axum `Query` extractor and automatically validates them.
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
