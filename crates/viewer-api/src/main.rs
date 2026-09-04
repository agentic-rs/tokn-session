use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};
use tokn_viewer_core::{
  ViewerService,
  relay::{RelayMode, RelaySettings},
  runtime::{ViewerRuntime, session_index_path},
};

#[derive(Parser)]
#[command(about = "Viewer web server and HTTP API")]
struct Args {
  #[command(subcommand)]
  command: Option<Command>,
  #[arg(long, default_value = "127.0.0.1:5558")]
  bind: SocketAddr,
  /// Required for non-loopback binding. Sent by clients as a Bearer token.
  #[arg(long, env = "TOKN_VIEWER_TOKEN", hide_env_values = true)]
  token: Option<String>,
  /// Exact browser origins allowed to read this API (repeatable).
  #[arg(long)]
  allow_origin: Vec<String>,
  /// Directory containing the compiled viewer index.html and assets.
  #[arg(long, env = "TOKN_VIEWER_WEB_ROOT", default_value = "apps/viewer/dist")]
  web_root: PathBuf,
  /// Serve only the API while Vite handles the UI and hot module replacement.
  #[arg(long)]
  api_only: bool,
  #[arg(long)]
  index_path: Option<PathBuf>,
  #[arg(long)]
  native: bool,
  /// Read provider history directly without a managed Relay child.
  #[arg(long)]
  local: bool,
}
#[derive(Subcommand)]
enum Command {
  /// Legacy desktop snapshot/follow transport, now owned by viewer-core.
  Snapshot {
    #[arg(long, default_value = "tcp://127.0.0.1:5557")]
    bind: String,
    #[arg(long)]
    native: bool,
  },
}
fn main() {
  tokn_session_relay::stdio::run_if_requested();
  let args = Args::parse();
  if let Err(error) = tokio::runtime::Runtime::new().unwrap().block_on(run(args)) {
    eprintln!("{error}");
    std::process::exit(1);
  }
}
async fn run(args: Args) -> Result<(), String> {
  if let Some(Command::Snapshot { bind, native }) = args.command {
    let mut config = tokn_session_relay::stdio::default_config(native)?;
    config.poll_interval = std::time::Duration::from_millis(500);
    return tokn_viewer_core::service_server::serve(&bind, config).await;
  }
  let token = args.token.filter(|token| !token.is_empty());
  if !args.bind.ip().is_loopback() && token.is_none() {
    return Err("Non-loopback binding requires TOKN_VIEWER_TOKEN or --token".into());
  }
  let origins = args
    .allow_origin
    .iter()
    .map(|origin| {
      if origin == "*" || origin == "null" {
        return Err("Use an exact browser origin".to_string());
      }
      origin.parse().map_err(|_| "Invalid browser origin".to_string())
    })
    .collect::<Result<Vec<_>, _>>()?;
  let path = match args.index_path {
    Some(path) => path,
    None => session_index_path()?,
  };
  if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  let service = ViewerService::native(path)?;
  service.relay.configure(RelaySettings {
    mode: if args.local {
      RelayMode::Local
    } else {
      RelayMode::Automatic
    },
    include_native: args.native,
    ..Default::default()
  })?;
  let runtime = ViewerRuntime::start(service.clone());
  let shutdown = tokio_util::sync::CancellationToken::new();
  let app = tokn_viewer_api::router(
    service.clone(),
    runtime.events.clone(),
    token,
    origins,
    shutdown.clone(),
  );
  let app = if args.api_only {
    app
  } else {
    tokn_viewer_api::with_web_ui(app, args.web_root)?
  };
  let listener = tokio::net::TcpListener::bind(args.bind)
    .await
    .map_err(|e| e.to_string())?;
  eprintln!(
    "Viewer listening on http://{}",
    listener.local_addr().map_err(|e| e.to_string())?
  );
  let result = axum::serve(listener, app)
    .with_graceful_shutdown(async move {
      let _ = tokio::signal::ctrl_c().await;
      shutdown.cancel();
    })
    .await
    .map_err(|e| e.to_string());
  service.relay.shutdown().await;
  drop(runtime);
  result
}
