#![forbid(unsafe_code)]
#![deny(elided_lifetimes_in_paths)]
#![deny(unreachable_pub)]

use cfg_if::cfg_if;

cfg_if! { if #[cfg(feature = "ssr")] {
    use std::process;
    use dashmap::DashMap;
    use block_mesh_common::interfaces::server_api::{CheckTokenResponseMap, GetTokenResponseMap};
    use std::mem;
    use logger_general::tracing::setup_tracing_stdout_only_with_sentry;
    use block_mesh_common::interfaces::ws_api::WsServerMessage;
    use block_mesh_manager::ws::ws_keep_alive::ws_keep_alive;
    use block_mesh_manager::database::user::create_test_user::create_test_user;
    use block_mesh_manager::ws::connection_manager::ConnectionManager;
    use block_mesh_manager::worker::analytics_agg::analytics_agg;
    use block_mesh_common::interfaces::db_messages::{
        AggregateMessage, AnalyticsMessage, DailyStatMessage,
    };
    use block_mesh_manager::worker::aggregate_agg::aggregate_agg;
    use block_mesh_common::interfaces::db_messages::UsersIpMessage;
    use block_mesh_manager::worker::users_ip_agg::users_ip_agg;
    use block_mesh_common::env::app_env_var::AppEnvVar;
    use block_mesh_common::env::env_var::EnvVar;
    use block_mesh_common::env::get_env_var_or_panic::get_env_var_or_panic;
    use block_mesh_common::env::load_dotenv::load_dotenv;
    use std::env;
    use block_mesh_manager::worker::daily_stat_agg::{daily_stat_agg};
    #[allow(unused_imports)]
    use logger_general::tracing::setup_tracing_stdout_only;
    use std::time::Duration;
    use reqwest::ClientBuilder;
    use block_mesh_manager::worker::db_cleaner_cron::{db_cleaner_cron, EnrichIp};
    use block_mesh_common::feature_flag_client::get_all_flags;
    use tokio::task::JoinHandle;
    use block_mesh_manager::worker::joiner::joiner_loop;
    #[cfg(not(target_env = "msvc"))]
    use tikv_jemallocator::Jemalloc;
    #[cfg(not(target_env = "msvc"))]
    #[global_allocator]
    static GLOBAL: Jemalloc = Jemalloc;
    use block_mesh_manager::configuration::get_configuration::get_configuration;
    use block_mesh_manager::database::migrate::migrate;
    use block_mesh_manager::emails::email_client::EmailClient;
    use block_mesh_manager::startup::application::{AppState, Application};
    use block_mesh_manager::startup::get_connection_pool::get_connection_pool;
    use secret::Secret;
    use std::sync::Arc;
}}

#[cfg(feature = "ssr")]
fn main() {
    let sentry_layer = env::var("SENTRY_LAYER")
        .unwrap_or("false".to_string())
        .parse()
        .unwrap_or(false);
    let sentry_url = env::var("SENTRY").unwrap_or_default();
    let sentry_sample_rate = env::var("SENTRY_SAMPLE_RATE")
        .unwrap_or("0.1".to_string())
        .parse()
        .unwrap_or(0.1);
    if sentry_layer {
        let _guard = sentry::init((
            sentry_url,
            sentry::ClientOptions {
                debug: env::var("APP_ENVIRONMENT").unwrap_or_default() == "local",
                sample_rate: sentry_sample_rate,
                traces_sample_rate: sentry_sample_rate,
                release: sentry::release_name!(),
                ..Default::default()
            },
        ));
        mem::forget(_guard);
    }

    let _ = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { run().await });
    process::exit(1);
}

#[cfg(feature = "ssr")]
async fn run() -> anyhow::Result<()> {
    load_dotenv();
    // setup_tracing_stdout_only();
    // console_subscriber::init(); // tokio-console
    setup_tracing_stdout_only_with_sentry();
    let configuration = get_configuration().expect("Failed to read configuration");
    tracing::info!("Starting with configuration {:#?}", configuration);
    let database_url = get_env_var_or_panic(AppEnvVar::DatabaseUrl);
    let database_url = <EnvVar as AsRef<Secret<String>>>::as_ref(&database_url);
    let mailgun_token = get_env_var_or_panic(AppEnvVar::MailgunSendKey);
    let _mailgun_token = <EnvVar as AsRef<Secret<String>>>::as_ref(&mailgun_token);
    let db_pool = get_connection_pool(&configuration.database, Option::from(database_url)).await?;
    migrate(&db_pool).await.expect("Failed to migrate database");
    let email_client = Arc::new(EmailClient::new(configuration.application.base_url.clone()).await);
    let (tx, rx) = flume::bounded::<JoinHandle<()>>(500);
    let (tx_daily_stat_agg, rx_daily_stat_agg) = flume::bounded::<DailyStatMessage>(500);
    let (tx_analytics_agg, rx_analytics_agg) = flume::bounded::<AnalyticsMessage>(500);
    let (tx_users_ip_agg, rx_users_ip_agg) = flume::bounded::<UsersIpMessage>(500);
    let (tx_aggregate_agg, rx_aggregate_agg) = flume::bounded::<AggregateMessage>(500);
    let (cleaner_tx, cleaner_rx) = flume::bounded::<EnrichIp>(500);
    let client = ClientBuilder::new()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let flags = get_all_flags(&client).await?;
    let redis_url = env::var("REDIS_URL")?;
    let redis_url = if redis_url.ends_with("#insecure") {
        redis_url
    } else {
        format!("{}#insecure", redis_url)
    };
    let redis_client = redis::Client::open(redis_url)?;
    let redis = redis_client.get_multiplexed_async_connection().await?;

    let _ = create_test_user(&db_pool).await;

    let mut ws_connection_manager = ConnectionManager::new();
    let broadcaster = ws_connection_manager.broadcaster.clone();
    let _ = ws_connection_manager
        .broadcaster
        .cron_reports(
            Duration::from_secs(60),
            vec![
                WsServerMessage::RequestUptimeReport,
                WsServerMessage::RequestBandwidthReport,
            ],
            100,
            db_pool.clone(),
        )
        .await;

    let check_token_map: CheckTokenResponseMap = Arc::new(DashMap::new());
    let get_token_map: GetTokenResponseMap = Arc::new(DashMap::new());

    let app_state = Arc::new(AppState {
        check_token_map,
        get_token_map,
        email_client,
        pool: db_pool.clone(),
        client,
        tx,
        tx_daily_stat_agg,
        tx_analytics_agg,
        flags,
        cleaner_tx,
        redis,
        ws_connection_manager,
        tx_users_ip_agg,
        tx_aggregate_agg,
    });

    let application = Application::build(configuration, app_state.clone(), db_pool.clone()).await;
    let application_task = tokio::spawn(application.run());
    let joiner_task = tokio::spawn(joiner_loop(rx));
    let db_cleaner_task = tokio::spawn(db_cleaner_cron(db_pool.clone(), cleaner_rx));
    let db_daily_stat_task = tokio::spawn(daily_stat_agg(
        db_pool.clone(),
        rx_daily_stat_agg,
        app_state.clone(),
    ));
    let db_analytics_task = tokio::spawn(analytics_agg(
        db_pool.clone(),
        rx_analytics_agg,
        app_state.clone(),
    ));
    let db_users_ip_task = tokio::spawn(users_ip_agg(
        db_pool.clone(),
        rx_users_ip_agg,
        app_state.clone(),
    ));
    let db_aggregate_task = tokio::spawn(aggregate_agg(
        db_pool.clone(),
        rx_aggregate_agg,
        app_state.clone(),
    ));
    let ws_ping_task = tokio::spawn(ws_keep_alive(broadcaster));

    tokio::select! {
        o = application_task => panic!("API {:?}", o),
        o = joiner_task => panic!("Joiner task failed {:?}", o),
        o = db_cleaner_task => panic!("DB cleaner task failed {:?}", o),
        o = db_daily_stat_task => panic!("DB daily_stat aggregator {:?}", o),
        o = db_analytics_task => panic!("DB analytics aggregator {:?}", o),
        o = db_users_ip_task => panic!("DB users_ip aggregator {:?}", o),
        o = db_aggregate_task => panic!("DB aggregate aggregator {:?}", o),
        o = ws_ping_task => panic!("ws_ping_task failed {:?}", o)
    }
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for a purely client-side app
    // see lib.rs for hydration function instead
}
