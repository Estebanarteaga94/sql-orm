use chrono::{DateTime, FixedOffset, NaiveTime};
use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "scheduled_jobs", schema = "dbo")]
pub struct ScheduledJob {
    #[orm(primary_key)]
    pub id: i64,
    pub run_at: NaiveTime,
    pub observed_at: DateTime<FixedOffset>,
}

fn main() {
    let metadata = ScheduledJob::metadata();
    assert_eq!(metadata.columns[1].sql_type, SqlServerType::Time);
    assert_eq!(
        metadata.columns[2].sql_type,
        SqlServerType::DateTimeOffset
    );
}
