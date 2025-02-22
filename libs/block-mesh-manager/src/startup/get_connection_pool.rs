use crate::configuration::database_settings::DatabaseSettings;
use anyhow::anyhow;
use secret::Secret;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool};
use std::str::FromStr;
use std::time::Duration;
use std::{env, thread, time};
use tracing::log;

#[tracing::instrument(name = "get_connection_pool", skip_all, ret, err)]
pub async fn get_connection_pool(
    settings: &DatabaseSettings,
    database_url: Option<&Secret<String>>,
) -> Result<PgPool, anyhow::Error> {
    let mut retries = 1;
    let settings: PgConnectOptions = match database_url {
        None => settings.with_db(),
        Some(database_url) => PgConnectOptions::from_str(database_url.as_ref())?
            .log_statements(log::LevelFilter::Trace)
            .options([
                (
                    "statement_timeout",
                    env::var("statement_timeout").unwrap_or("0".to_string()),
                ),
                (
                    "idle_in_transaction_session_timeout",
                    env::var("idle_in_transaction_session_timeout").unwrap_or("3000ms".to_string()),
                ),
                (
                    "lock_timeout",
                    env::var("lock_timeout").unwrap_or("1500ms".to_string()),
                ),
            ])
            .clone(),
    };

    loop {
        tracing::info!("attempting to connect to database - retry : {}", retries);
        if retries > 10 {
            return Err(anyhow!(
                "failed to connect to database after retries : {}",
                retries
            ));
        }
        let pool_connection = PgPoolOptions::new()
            .acquire_timeout(Duration::from_secs(
                env::var("ACQUIRE_TIMEOUT")
                    .unwrap_or("35".to_string())
                    .parse()
                    .unwrap_or(35),
            ))
            .min_connections(1)
            .max_connections(
                env::var("MAX_CONNECTIONS")
                    .unwrap_or("35".to_string())
                    .parse()
                    .unwrap_or(35),
            )
            .idle_timeout(Duration::from_millis(
                env::var("IDLE_TIMEOUT")
                    .unwrap_or("500".to_string())
                    .parse()
                    .unwrap_or(500),
            ))
            .max_lifetime(Duration::from_millis(
                env::var("MAX_LIFETIME")
                    .unwrap_or("30000".to_string())
                    .parse()
                    .unwrap_or(30000),
            ))
            .test_before_acquire(true)
            .connect_with(settings.clone())
            .await;
        match pool_connection {
            Ok(pool) => {
                tracing::info!("connected to database - retry : {}", retries);
                return Ok(pool);
            }
            Err(error) => {
                tracing::warn!(
                    "failed to connect to database - retry : {} - error {:#?}",
                    retries,
                    error
                );
                let ten_millis = time::Duration::from_millis(1_000);
                thread::sleep(ten_millis);
                retries += 1;
            }
        }
    }
}
