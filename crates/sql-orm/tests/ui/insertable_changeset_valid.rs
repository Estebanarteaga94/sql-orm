use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
struct Customer {
    id: i64,
    email: String,
    phone: Option<String>,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = Customer)]
struct NewCustomer {
    email: String,
    phone: Option<String>,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = Customer)]
struct UpdateCustomer {
    email: Option<String>,
    phone: Option<Option<String>>,
}

fn main() {
    let new_customer = NewCustomer {
        email: "ana@example.com".to_string(),
        phone: None,
    };
    let values = <NewCustomer as Insertable<Customer>>::values(&new_customer);
    assert_eq!(values.len(), 2);

    let update = UpdateCustomer {
        email: Some("ana.maria@example.com".to_string()),
        phone: Some(None),
    };
    let changes = <UpdateCustomer as Changeset<Customer>>::changes(&update);
    assert_eq!(changes.len(), 2);
}
