//! Headless entry point in the shipped viewer executable. No Tauri windows or
//! index workers are initialized in this process; the implementation is Relay's
//! existing snapshot service, not a second server implementation.
use std::io::{Read, Write};
use std::time::Duration;

use tokn_session_client::{AgentClient, Source};
use tokn_session_core::Provider;
use tokn_session_opencode::OpenCodeSessionSource;
use tokn_session_relay::{ProviderRoot, RelayConfig, service_server::serve_listener};

pub(crate) const CHILD_FLAG: &str = "--tokn-viewer-relay-child";

pub fn run_if_requested() {
  let mut args = std::env::args_os().skip(1);
  if args.next().as_deref() != Some(std::ffi::OsStr::new(CHILD_FLAG)) {
    return;
  }
  let native = match (args.next(), args.next()) {
    (None, None) => false,
    (Some(flag), None) if flag == "--native" => true,
    _ => std::process::exit(2),
  };
  // A dedicated blocking reader survives a stalled Tokio runtime. The parent
  // exclusively owns the write end: normal shutdown or parent death closes it.
  // This read-only child must not linger waiting for provider blocking tasks.
  std::thread::spawn(|| {
    let mut byte = [0];
    let mut stdin = std::io::stdin().lock();
    while matches!(stdin.read(&mut byte), Ok(1)) {}
    std::process::exit(0);
  });
  let result = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .map_err(|e| e.to_string())
    .and_then(|runtime| runtime.block_on(serve(native)));
  if let Err(error) = result {
    let _ = report_ready(Err(error));
    std::process::exit(1);
  }
  std::process::exit(0);
}

fn config(native: bool) -> Result<RelayConfig, String> {
  let mut roots = Vec::new();
  for (source, provider) in [(Source::Codex, Provider::Codex), (Source::Pi, Provider::Pi)] {
    roots.extend(
      AgentClient::file_session_roots(source, None)?
        .into_iter()
        .map(|path| ProviderRoot::new(provider, path)),
    );
  }
  roots.push(ProviderRoot::new(
    Provider::OpenCode,
    OpenCodeSessionSource::new(None).database_path()?,
  ));
  let mut config = RelayConfig::new(roots);
  config.include_native = native;
  config.poll_interval = Duration::from_millis(500);
  Ok(config)
}

fn report_ready(result: Result<String, String>) -> Result<(), String> {
  let mut stdout = std::io::stdout().lock();
  serde_json::to_writer(&mut stdout, &result).map_err(|e| e.to_string())?;
  writeln!(stdout).and_then(|_| stdout.flush()).map_err(|e| e.to_string())
}

async fn serve(native: bool) -> Result<(), String> {
  let config = config(native)?;
  // Bind once and retain the listener. No find-free-port / rebind race and no
  // conflict with an independent Relay (or another viewer instance).
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
    .await
    .map_err(|e| e.to_string())?;
  report_ready(Ok(format!(
    "tcp://{}",
    listener.local_addr().map_err(|e| e.to_string())?
  )))?;
  serve_listener(listener, config).await
}
