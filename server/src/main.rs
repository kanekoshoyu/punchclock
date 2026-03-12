use punchclock_server::{AppState, ServerConfig, SharedState, build_app, reaper};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env"));

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "punchclock_server=info,poem=info".into()),
        )
        .init();

    let api_base_url = std::env::var("API_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8421".to_string());

    let port: u16 = api_base_url
        .trim_end_matches('/')
        .rsplit(':')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8421);

    let state: SharedState = Arc::new(AppState::new(ServerConfig::default()));
    tokio::spawn(reaper(state.clone()));

    let app = build_app(state.clone(), &api_base_url);

    let addr = format!("0.0.0.0:{port}");
    tracing::info!("punchclock listening on {addr}  docs → {api_base_url}/docs");
    poem::Server::new(poem::listener::TcpListener::bind(addr))
        .run(app)
        .await?;
    Ok(())
}
