use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokn_session_core::Provider;
use tokn_session_relay::{ProviderRoot, SessionTailer, TailUpdate, ZmqPublisher};

const DEFAULT_ENDPOINT: &str = "tcp://127.0.0.1:5556";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() {
  if let Err(err) = run().await {
    eprintln!("error: {err}");
    std::process::exit(1);
  }
}

async fn run() -> Result<(), String> {
  let args = Args::parse(std::env::args().skip(1))?;
  if args.help {
    print_help();
    return Ok(());
  }

  let roots = args.roots()?;
  let (mut tailer, initial) = SessionTailer::initialize(roots, args.replay)?;
  let mut publisher = ZmqPublisher::bind(&args.endpoint).await?;
  eprintln!("publishing Codex/Pi session events on {}", args.endpoint);
  publish_update(&mut publisher, initial).await;

  let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();
  let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
    let _ = wake_tx.send(result.map(|_| ()));
  })
  .map_err(|err| format!("failed to create filesystem watcher: {err}"))?;
  for root in tailer.roots() {
    if root.path.exists() {
      watcher
        .watch(&root.path, RecursiveMode::Recursive)
        .map_err(|err| format!("failed to watch {}: {err}", root.path.display()))?;
    }
  }

  let mut poll = tokio::time::interval(DEFAULT_POLL_INTERVAL);
  poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
  loop {
    tokio::select! {
      _ = poll.tick() => {}
      wake = wake_rx.recv() => {
        match wake {
          Some(Ok(())) => {}
          Some(Err(err)) => eprintln!("warning: filesystem watcher error: {err}"),
          None => return Err("filesystem watcher stopped unexpectedly".to_string()),
        }
      }
      signal = tokio::signal::ctrl_c() => {
        signal.map_err(|err| format!("failed to listen for shutdown signal: {err}"))?;
        return Ok(());
      }
    }

    match tailer.scan() {
      Ok(update) => publish_update(&mut publisher, update).await,
      Err(err) => eprintln!("warning: {err}"),
    }
  }
}

async fn publish_update(publisher: &mut ZmqPublisher, update: TailUpdate) {
  for warning in update.warnings {
    eprintln!("warning: {warning}");
  }
  for event in update.events {
    if let Err(err) = publisher.publish(&event.topic, &event.event).await {
      eprintln!("warning: {err}");
    }
  }
}

struct Args {
  endpoint: String,
  codex_dir: Option<PathBuf>,
  pi_dir: Option<PathBuf>,
  replay: bool,
  help: bool,
}

impl Args {
  fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
    let mut parsed = Self {
      endpoint: DEFAULT_ENDPOINT.to_string(),
      codex_dir: None,
      pi_dir: None,
      replay: false,
      help: false,
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
      match arg.as_str() {
        "--bind" => parsed.endpoint = next_value(&mut args, "--bind")?,
        "--codex-dir" => parsed.codex_dir = Some(PathBuf::from(next_value(&mut args, "--codex-dir")?)),
        "--pi-dir" => parsed.pi_dir = Some(PathBuf::from(next_value(&mut args, "--pi-dir")?)),
        "--replay" => parsed.replay = true,
        "-h" | "--help" => parsed.help = true,
        _ => return Err(format!("unknown argument `{arg}`; use --help for usage")),
      }
    }
    Ok(parsed)
  }

  fn roots(&self) -> Result<Vec<ProviderRoot>, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let codex = resolve_root(&self.codex_dir, home.as_deref(), ".codex/sessions")?;
    let pi = resolve_root(&self.pi_dir, home.as_deref(), ".pi/agent/sessions")?;
    Ok(vec![
      ProviderRoot::new(Provider::Codex, codex),
      ProviderRoot::new(Provider::Pi, pi),
    ])
  }
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
  args.next().ok_or_else(|| format!("{flag} requires a value"))
}

fn resolve_root(explicit: &Option<PathBuf>, home: Option<&Path>, relative: &str) -> Result<PathBuf, String> {
  explicit
    .clone()
    .or_else(|| home.map(|home| home.join(relative)))
    .ok_or_else(|| format!("HOME is not set; pass an explicit directory for {relative}"))
}

fn print_help() {
  println!(
    "\
Publish newly appended Codex and Pi session events over ZeroMQ.

Usage:
  tokn-session-relay [options]

Options:
  --bind <endpoint>    PUB endpoint (default: {DEFAULT_ENDPOINT})
  --codex-dir <path>   Codex session root (default: ~/.codex/sessions)
  --pi-dir <path>      Pi session root (default: ~/.pi/agent/sessions)
  --replay             Publish existing complete records before following
  -h, --help           Show this help

Messages use two ZeroMQ frames:
  1. topic: codex.<session_id> or pi.<session_id>
  2. JSON: normalized AgentEvent"
  );
}
