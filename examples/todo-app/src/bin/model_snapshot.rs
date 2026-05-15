use todo_app::TodoAppDbContext;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    print!(
        "{}",
        sql_orm::model_snapshot_json_from_source::<TodoAppDbContext>()?
    );
    Ok(())
}
