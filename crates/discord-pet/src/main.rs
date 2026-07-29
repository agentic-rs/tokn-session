use std::path::{Path, PathBuf};

use tokn_discord_pet::{DiscordConfig, DiscordPet, default_config_path, login, permissions_warning, state_path};
use tokn_session_core::Provider;
use tokn_session_relay::{NewFileReplay, ProviderRoot, RelayConfig, SessionRelay};

#[tokio::main]
async fn main() {
  match Args::parse(std::env::args().skip(1)) {
    Ok(ArgsParse::Run(command)) => {
      let result = match command {
        Command::Run(args) => run(args).await,
        Command::Login(args) => login_command(args).await,
      };
      if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
      }
    }
    Ok(ArgsParse::Help(help)) => print_help(help),
    Err(err) => {
      eprintln!("error: {err}");
      std::process::exit(2);
    }
  }
}

async fn login_command(args: LoginArgs) -> Result<(), String> {
  let config_path = args.config.unwrap_or(default_config_path()?);
  login(&config_path).await
}

async fn run(args: RunArgs) -> Result<(), String> {
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

struct RunArgs {
  config: Option<PathBuf>,
  codex_dir: Option<PathBuf>,
  pi_dir: Option<PathBuf>,
}

struct LoginArgs {
  config: Option<PathBuf>,
}

enum Command {
  Run(RunArgs),
  Login(LoginArgs),
}

enum ArgsParse {
  Run(Command),
  Help(Help),
}

#[derive(Clone, Copy)]
enum Help {
  Root,
  Run,
  Login,
}

struct Args;

impl Args {
  fn parse(args: impl Iterator<Item = String>) -> Result<ArgsParse, String> {
    let mut args = args.peekable();
    match args.peek().map(String::as_str) {
      Some("login") => {
        args.next();
        Self::parse_login(args)
      }
      Some("run") => {
        args.next();
        Self::parse_run(args)
      }
      Some("-h" | "--help") => Ok(ArgsParse::Help(Help::Root)),
      _ => Self::parse_run(args),
    }
  }

  fn parse_run(mut args: impl Iterator<Item = String>) -> Result<ArgsParse, String> {
    let mut parsed = RunArgs {
      config: None,
      codex_dir: None,
      pi_dir: None,
    };
    while let Some(arg) = args.next() {
      match arg.as_str() {
        "--config" => parsed.config = Some(PathBuf::from(next_value(&mut args, "--config")?)),
        "--codex-dir" => parsed.codex_dir = Some(PathBuf::from(next_value(&mut args, "--codex-dir")?)),
        "--pi-dir" => parsed.pi_dir = Some(PathBuf::from(next_value(&mut args, "--pi-dir")?)),
        "-h" | "--help" => return Ok(ArgsParse::Help(Help::Run)),
        _ => return Err(format!("unknown argument `{arg}`; use --help for usage")),
      }
    }
    Ok(ArgsParse::Run(Command::Run(parsed)))
  }

  fn parse_login(mut args: impl Iterator<Item = String>) -> Result<ArgsParse, String> {
    let mut parsed = LoginArgs { config: None };
    while let Some(arg) = args.next() {
      match arg.as_str() {
        "--config" => parsed.config = Some(PathBuf::from(next_value(&mut args, "--config")?)),
        "-h" | "--help" => return Ok(ArgsParse::Help(Help::Login)),
        _ => return Err(format!("unknown login argument `{arg}`; use `login --help` for usage")),
      }
    }
    Ok(ArgsParse::Run(Command::Login(parsed)))
  }
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
  args.next().ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help(help: Help) {
  match help {
    Help::Root => println!(
      "\
Publish root Codex and Pi user/final messages into Discord session threads.

Usage:
  tokn-discord-pet [run] [options]
  tokn-discord-pet login [options]

Commands:
  run    Follow sessions (default when omitted)
  login  Interactively configure and validate Discord credentials

Run `tokn-discord-pet <command> --help` for command options."
    ),
    Help::Run => println!(
      "\
Follow sessions and publish root user/final messages.

Usage:
  tokn-discord-pet [run] [options]

Options:
  --config <path>     Config file (default: ~/.tokn/pet/discord.yaml)
  --codex-dir <path>  Codex session root (default: ~/.codex/sessions)
  --pi-dir <path>     Pi session root (default: ~/.pi/agent/sessions)
  -h, --help          Show this help"
    ),
    Help::Login => println!(
      "\
Interactively configure and validate Discord credentials.

Usage:
  tokn-discord-pet login [options]

Options:
  --config <path>  Config file (default: ~/.tokn/pet/discord.yaml)
  -h, --help       Show this help"
    ),
  }
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use super::{Args, ArgsParse, Command, Help};

  #[test]
  fn parses_login_and_config_override() {
    let ArgsParse::Run(Command::Login(args)) = Args::parse(
      ["login", "--config", "/tmp/discord.yaml"]
        .into_iter()
        .map(str::to_string),
    )
    .unwrap() else {
      panic!("expected login command");
    };
    assert_eq!(args.config, Some(PathBuf::from("/tmp/discord.yaml")));
  }

  #[test]
  fn keeps_implicit_and_explicit_run_forms() {
    assert!(matches!(
      Args::parse(std::iter::empty()).unwrap(),
      ArgsParse::Run(Command::Run(_))
    ));
    assert!(matches!(
      Args::parse(["run".to_string()].into_iter()).unwrap(),
      ArgsParse::Run(Command::Run(_))
    ));
  }

  #[test]
  fn rejects_run_only_options_for_login() {
    assert!(Args::parse(["login", "--codex-dir", "/tmp/codex"].into_iter().map(str::to_string)).is_err());
  }

  #[test]
  fn routes_command_help() {
    assert!(matches!(
      Args::parse(["--help".to_string()].into_iter()).unwrap(),
      ArgsParse::Help(Help::Root)
    ));
    assert!(matches!(
      Args::parse(["run", "--help"].into_iter().map(str::to_string)).unwrap(),
      ArgsParse::Help(Help::Run)
    ));
    assert!(matches!(
      Args::parse(["login", "--help"].into_iter().map(str::to_string)).unwrap(),
      ArgsParse::Help(Help::Login)
    ));
  }
}
