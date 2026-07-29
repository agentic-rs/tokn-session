use std::path::{Path, PathBuf};

use tokn_discord_pet::{DiscordConfig, DiscordPet, default_config_path, permissions_warning, state_path};
use tokn_session_core::Provider;
use tokn_session_relay::{NewFileReplay, ProviderRoot, RelayConfig, SessionRelay};

#[tokio::main]
async fn main() {
  match Args::parse(std::env::args().skip(1)) {
    Ok(ArgsParse::Run(args)) => {
      if let Err(err) = run(args).await {
        eprintln!("error: {err}");
        std::process::exit(1);
      }
    }
    Ok(ArgsParse::Help) => print_help(),
    Err(err) => {
      eprintln!("error: {err}");
      std::process::exit(2);
    }
  }
}

async fn run(args: Args) -> Result<(), String> {
  let config_path = args.config.unwrap_or(default_config_path()?);
  let config = DiscordConfig::load(&config_path)?;
  if let Some(warning) = permissions_warning(&config_path) {
    eprintln!("warning: {warning}");
  }
  let mut api = tokn_discord_pet::DiscordClient::new(&config.bot_token)?;
  let username = api.validate_destination(&config.guild_id, &config.channel_id).await?;
  let roots = provider_roots(args.codex_dir.as_deref(), args.pi_dir.as_deref())?;
  let relay_config = RelayConfig {
    roots,
    poll_interval: tokn_session_relay::DEFAULT_POLL_INTERVAL,
    new_file_replay: NewFileReplay::Messages(3),
  };
  let mut relay = SessionRelay::new(relay_config).await?;
  let mut pet = DiscordPet::new(api, config.channel_id, state_path(&config_path))?;
  eprintln!("Discord pet @{username} is following root Codex/Pi sessions");

  loop {
    tokio::select! {
      update = relay.next_update() => {
        let update = update?;
        for warning in update.warnings {
          eprintln!("warning: {warning}");
        }
        for event in update.events {
          if let Err(err) = pet.process(&event).await {
            eprintln!("warning: failed to publish {}: {err}", event.topic);
          }
        }
      }
      signal = tokio::signal::ctrl_c() => {
        signal.map_err(|err| format!("failed to listen for shutdown signal: {err}"))?;
        return Ok(());
      }
    }
  }
}

fn provider_roots(codex_dir: Option<&Path>, pi_dir: Option<&Path>) -> Result<Vec<ProviderRoot>, String> {
  let home = std::env::var_os("HOME").map(PathBuf::from);
  let codex = codex_dir
    .map(Path::to_path_buf)
    .or_else(|| home.as_deref().map(|home| home.join(".codex/sessions")))
    .ok_or_else(|| "HOME is not set; pass `--codex-dir <path>`".to_string())?;
  let pi = pi_dir
    .map(Path::to_path_buf)
    .or_else(|| home.as_deref().map(|home| home.join(".pi/agent/sessions")))
    .ok_or_else(|| "HOME is not set; pass `--pi-dir <path>`".to_string())?;
  Ok(vec![
    ProviderRoot::new(Provider::Codex, codex),
    ProviderRoot::new(Provider::Pi, pi),
  ])
}

struct Args {
  config: Option<PathBuf>,
  codex_dir: Option<PathBuf>,
  pi_dir: Option<PathBuf>,
}

enum ArgsParse {
  Run(Args),
  Help,
}

impl Args {
  fn parse(mut args: impl Iterator<Item = String>) -> Result<ArgsParse, String> {
    let mut parsed = Self {
      config: None,
      codex_dir: None,
      pi_dir: None,
    };
    while let Some(arg) = args.next() {
      match arg.as_str() {
        "--config" => parsed.config = Some(PathBuf::from(next_value(&mut args, "--config")?)),
        "--codex-dir" => parsed.codex_dir = Some(PathBuf::from(next_value(&mut args, "--codex-dir")?)),
        "--pi-dir" => parsed.pi_dir = Some(PathBuf::from(next_value(&mut args, "--pi-dir")?)),
        "-h" | "--help" => return Ok(ArgsParse::Help),
        _ => return Err(format!("unknown argument `{arg}`; use --help for usage")),
      }
    }
    Ok(ArgsParse::Run(parsed))
  }
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
  args.next().ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help() {
  println!(
    "\
Publish root Codex and Pi user/final messages into Discord session threads.

Usage:
  tokn-discord-pet [options]

Options:
  --config <path>     Config file (default: ~/.tokn/pet/discord.yaml)
  --codex-dir <path>  Codex session root (default: ~/.codex/sessions)
  --pi-dir <path>     Pi session root (default: ~/.pi/agent/sessions)
  -h, --help          Show this help"
  );
}
