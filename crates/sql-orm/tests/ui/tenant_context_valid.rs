use sql_orm::prelude::*;

#[derive(TenantContext)]
struct CurrentTenant {
    #[orm(column = "company_id")]
    tenant_id: i64,
}

fn main() {
    let metadata = CurrentTenant::metadata();
    let tenant = CurrentTenant { tenant_id: 42 };

    assert_eq!(metadata.name, "tenant");
    assert_eq!(metadata.columns.len(), 1);
    assert_eq!(metadata.columns[0].rust_field, "tenant_id");
    assert_eq!(metadata.columns[0].column_name, "company_id");
    assert_eq!(metadata.columns[0].sql_type, SqlServerType::BigInt);
    assert!(metadata.columns[0].insertable);
    assert!(!metadata.columns[0].updatable);
    assert_eq!(CurrentTenant::COLUMN_NAME, "company_id");
    assert_eq!(tenant.tenant_value(), SqlValue::I64(42));
    assert_eq!(
        <CurrentTenant as EntityPolicy>::COLUMN_NAMES,
        &["company_id"]
    );
}
