use sql_orm::prelude::*;

#[derive(TenantContext)]
struct CurrentTenant {
    #[orm(column = "company_id")]
    tenant_id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "tenant_orders", schema = "sales", tenant = CurrentTenant)]
struct TenantOrder {
    #[orm(primary_key)]
    id: i64,

    #[orm(length = 120)]
    description: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "currencies", schema = "sales")]
struct Currency {
    #[orm(primary_key)]
    #[orm(length = 3)]
    code: String,

    #[orm(length = 80)]
    name: String,
}

fn main() {
    let tenant = TenantOrder::tenant_policy().expect("tenant policy");
    assert_eq!(tenant.name, "tenant");
    assert_eq!(tenant.columns.len(), 1);
    assert_eq!(tenant.columns[0].column_name, "company_id");

    let metadata = TenantOrder::metadata();
    assert_eq!(metadata.columns.len(), 3);
    assert_eq!(metadata.columns[0].column_name, "id");
    assert_eq!(metadata.columns[1].column_name, "description");
    assert_eq!(metadata.columns[2].column_name, "company_id");

    assert_eq!(Currency::tenant_policy(), None);
    assert_eq!(Currency::metadata().columns.len(), 2);
}
