//! Managed live feed: stdout is protocol-only, stderr carries diagnostics,
//! and stdin EOF ends the child even if its async runtime stalls.
use crate::{PROVIDERS, RelayConfig, SessionRelay, provider_roots};
use std::io::{Read, Write};

pub const CHILD_FLAG: &str = "--tokn-viewer-relay-child";
pub const VERSION: u32 = 1;
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

pub fn default_config(native: bool) -> Result<RelayConfig, String> {
  let mut roots = Vec::new();
  for provider in PROVIDERS {
    roots.extend(provider_roots(provider, None)?);
  }
  let mut config = RelayConfig::new(roots);
  config.include_native = native;
  Ok(config)
}

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
  std::thread::spawn(|| {
    let mut byte = [0];
    while matches!(std::io::stdin().read(&mut byte), Ok(1)) {}
    std::process::exit(0);
  });
  let result = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .map_err(|e| e.to_string())
    .and_then(|runtime| runtime.block_on(run(native)));
  if let Err(error) = result {
    eprintln!("Relay stopped: {error}");
    std::process::exit(1);
  }
  std::process::exit(0);
}

async fn run(native: bool) -> Result<(), String> {
  let mut relay = SessionRelay::new(default_config(native)?).await?;
  write_line(&serde_json::json!({"type":"ready", "version":VERSION}))?;
  loop {
    let update = relay.next_update().await?;
    for warning in update.warnings {
      eprintln!("{warning}");
    }
    for record in update.records {
      write_line(&record)?;
    }
  }
}

fn write_line(value: &impl serde::Serialize) -> Result<(), String> {
  let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
  if bytes.len() >= MAX_LINE_BYTES {
    return Err("Relay record exceeds pipe frame limit".into());
  }
  let mut stdout = std::io::stdout().lock();
  stdout
    .write_all(&bytes)
    .and_then(|_| stdout.write_all(b"\n"))
    .and_then(|_| stdout.flush())
    .map_err(|e| e.to_string())
}
