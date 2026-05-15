use sql_orm_query::Pagination;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    pub page: u64,
    pub page_size: u64,
}

impl PageRequest {
    pub const fn new(page: u64, page_size: u64) -> Self {
        Self { page, page_size }
    }

    pub const fn to_pagination(self) -> Pagination {
        Pagination::page(self.page, self.page_size)
    }
}

#[cfg(test)]
mod tests {
    use super::PageRequest;
    use sql_orm_query::Pagination;

    #[test]
    fn page_request_converts_to_pagination() {
        assert_eq!(
            PageRequest::new(1, 25).to_pagination(),
            Pagination::new(0, 25)
        );
        assert_eq!(
            PageRequest::new(3, 25).to_pagination(),
            Pagination::new(50, 25)
        );
    }
}
