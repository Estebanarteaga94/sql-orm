#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pagination {
    pub offset: u64,
    pub limit: u64,
}

impl Pagination {
    pub const fn new(offset: u64, limit: u64) -> Self {
        Self { offset, limit }
    }

    pub const fn page(page: u64, page_size: u64) -> Self {
        let offset = if page <= 1 { 0 } else { (page - 1) * page_size };
        Self::new(offset, page_size)
    }
}
