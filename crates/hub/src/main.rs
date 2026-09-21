use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};
use tokio_util::sync::CancellationToken;
use tokn_session_hub::{
  connector::{self, ConnectorConfig},
  server::{self, HubState},
  store::Store,
};
use url::Url;

#[derive(Parser)]
#[command(about = "Passwordless access to session hosts through one Hub")]
struct Args {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Serve the Hub API and viewer. Use HTTPS termination for remote access.
  Serve {
    #[arg(long, default_value = "127.0.0.1:5559")]
    bind: SocketAddr,
    /// Stable browser origin used by passkeys (HTTPS, or http://localhost for development).
    #[arg(long, default_value = "http://localhost:5559")]
    public_url: String,
    #[arg(long)]
    state_path: Option<PathBuf>,
    #[arg(long, default_value = "apps/viewer/dist")]
    web_root: PathBuf,
    #[arg(long)]
    api_only: bool,
  },
  /// Enroll this host and keep an outbound tunnel to its loopback viewer-api.
  Connect {
    #[arg(long)]
    hub: Url,
    #[arg(long)]
    name: String,
    #[arg(long, default_value = "http://127.0.0.1:5558")]
    viewer_url: Url,
    #[arg(long, env = "TOKN_VIEWER_TOKEN", hide_env_values = true)]
    viewer_token: Option<String>,
    /// Private generated host key. Reuse it when reconnecting to the same Hub.
    #[arg(long)]
    identity_file: Option<PathBuf>,
    /// Allow sending input to live agents in addition to viewing sessions.
    #[arg(long)]
    allow_control: bool,
    /// Allow an unencrypted Hub URL only on loopback, for local development.
    #[arg(long)]
    insecure_loopback: bool,
  },
}

#[tokio::main]
async fn main() {
  if let Err(error) = run(Args::parse()).await {
    eprintln!("{error}");
    std::process::exit(1);
  }
}

fn default_path(file: &str) -> Result<PathBuf, String> {
  dirs::home_dir()
    .map(|home| home.join(".tokn/hub").join(file))
    .ok_or_else(|| "Cannot resolve home directory; specify an explicit state/key path".into())
}

async fn run(args: Args) -> Result<(), String> {
  let shutdown = CancellationToken::new();
  match args.command {
    Command::Serve {
      bind,
      public_url,
      state_path,
      web_root,
      api_only,
    } => {
      let origin = Url::parse(&public_url).map_err(|e| e.to_string())?;
      if !bind.ip().is_loopback() && origin.scheme() != "https" {
        return Err("Non-loopback Hub binding requires an HTTPS --public-url and HTTPS termination".into());
      }
      let state_path = match state_path {
        Some(path) => path,
        None => default_path("state.sqlite")?,
      };
      let state = HubState::new(Store::open(state_path)?, &public_url)?;
      let app = server::router(state.clone());
      let app = if api_only {
        app
      } else {
        server::with_web_ui(app, web_root)?
      };
      let listener = tokio::net::TcpListener::bind(bind).await.map_err(|e| e.to_string())?;
      eprintln!(
        "Hub listening on {} (public origin: {public_url})",
        listener.local_addr().map_err(|e| e.to_string())?
      );
      if let Some(token) = state.auth.bootstrap_token()? {
        let mut setup = origin;
        setup.set_fragment(Some(&format!("bootstrap_token={token}")));
        eprintln!("Create the owner's first passkey: {setup}");
      }
      let cancellation = shutdown.clone();
      let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
          let _ = tokio::signal::ctrl_c().await;
          cancellation.cancel();
          state.auth.shutdown();
          state.tunnels.shutdown();
        })
        .await
        .map_err(|e| e.to_string());
      result
    }
    Command::Connect {
      hub,
      name,
      viewer_url,
      viewer_token,
      identity_file,
      allow_control,
      insecure_loopback,
    } => {
      let key_file = match identity_file {
        Some(path) => path,
        None => default_path("host.key")?,
      };
      let config = ConnectorConfig {
        hub_url: hub,
        local_url: viewer_url,
        key_file,
        name,
        local_token: viewer_token,
        allow_control,
        insecure_loopback,
      };
      let cancellation = shutdown.clone();
      let signal = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancellation.cancel();
      });
      let result = connector::run(config, shutdown).await;
      signal.abort();
      result
    }
  }
}
