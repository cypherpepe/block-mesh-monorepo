use block_mesh_common::constants::BLOCKMESH_PG_NOTIFY_WORKER;
use serde::Serialize;
use sqlx::PgPool;
use std::fmt::Debug;

#[tracing::instrument(name = "notify_worker", skip_all)]
pub async fn notify_worker<M>(pool: &PgPool, message: M) -> anyhow::Result<()>
where
    M: Serialize + Clone + Debug,
{
    let s = serde_json::to_string(&message)?.replace('\'', "\"");
    let q = format!("NOTIFY {BLOCKMESH_PG_NOTIFY_WORKER} , '{s}'");
    sqlx::query(&q).execute(pool).await?;
    Ok(())
}
