use std::net::SocketAddr;

use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use same_actor::build_app;
use same_actor::config::Config;
use same_actor::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let config = Config::from_env()?;
    let bind = config.bind;
    let trust_forwarded_for = config.trust_forwarded_for;
    let state = AppState::new(config)?;
    let app = build_app(state.clone());

    let listener = TcpListener::bind(bind).await?;
    tracing::info!(%bind, trust_forwarded_for, "same-actor listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Fold the WAL back in and stop the writer thread, so a restart or a backup never sees a
    // half-applied batch.
    if let Err(error) = state.store.close().await {
        tracing::warn!(%error, "could not close the store cleanly");
    }

    let stats = state.store.stats();
    tracing::info!(
        rows = stats.rows,
        commits = stats.commits,
        dropped = stats.dropped,
        "shut down cleanly"
    );
    Ok(())
}

/// Finish in-flight requests on SIGTERM/SIGINT instead of dropping connections — the old
/// container was killed mid-response on every deploy.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "could not install a SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("interrupt received"),
        () = terminate => tracing::info!("termination received"),
    }
}
