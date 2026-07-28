use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokn_session_core::Provider;
use tokn_session_relay::{
  DEFAULT_POLL_INTERVAL, DEFAULT_REPLAY_MESSAGES, NewFileReplay, ProviderRoot, RelayConfig, RelayEvent, SessionRelay,
  TailUpdate, ZmqPublisher,
};

const DEFAULT_ENDPOINT: &str = "tcp://127.0.0.1:5556";

#[tokio::main]
async fn main() {
  match Args::parse(std::env::args().skip(1)) {
    Ok(ArgsParse::Run(args)) => {
      if let Err(err) = run(args).await {
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

async fn run(args: Args) -> Result<(), String> {
  let config = RelayConfig {
    roots: args.roots()?,
    poll_interval: args.poll_interval,
    new_file_replay: args.new_file_replay,
  };
  let mut relay = SessionRelay::new(config).await?;
  let mut output = match args.command {
    Command::ZeroMq { endpoint } => {
      let publisher = ZmqPublisher::bind(&endpoint).await?;
      eprintln!("following Codex/Pi session events via ZeroMQ on {endpoint}");
      Output::ZeroMq(publisher)
    }
    Command::Stdout => {
      eprintln!("following Codex/Pi session events on stdout");
      Output::Stdout(BufWriter::new(std::io::stdout()))
    }
  };

  loop {
    tokio::select! {
      update = relay.next_update() => output.write_update(update?).await,
      signal = tokio::signal::ctrl_c() => {
        signal.map_err(|err| format!("failed to listen for shutdown signal: {err}"))?;
        return Ok(());
      }
    }
  }
}

enum Output {
  ZeroMq(ZmqPublisher),
  Stdout(BufWriter<std::io::Stdout>),
}

impl Output {
  async fn write_update(&mut self, update: TailUpdate) {
    for warning in update.warnings {
      eprintln!("warning: {warning}");
    }
    for event in update.events {
      let result = match self {
        Self::ZeroMq(publisher) => publisher.publish(&event).await,
        Self::Stdout(writer) => write_jsonl_event(writer, &event),
      };
      if let Err(err) = result {
        eprintln!("warning: {err}");
      }
    }
  }
}

fn write_jsonl_event(writer: &mut impl Write, event: &RelayEvent) -> Result<(), String> {
  serde_json::to_writer(&mut *writer, &event.event).map_err(|err| format!("failed to serialize relay event: {err}"))?;
  writer
    .write_all(b"\n")
    .and_then(|_| writer.flush())
    .map_err(|err| format!("failed to write relay event: {err}"))
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
  ZeroMq { endpoint: String },
  Stdout,
}

#[derive(Debug, Eq, PartialEq)]
struct Args {
  command: Command,
  codex_dir: Option<PathBuf>,
  pi_dir: Option<PathBuf>,
  poll_interval: Duration,
  new_file_replay: NewFileReplay,
}

enum ArgsParse {
  Run(Args),
  Help(Help),
}

#[derive(Clone, Copy)]
enum Help {
  Root,
  ZeroMq,
  Stdout,
}

impl Args {
  fn parse(mut args: impl Iterator<Item = String>) -> Result<ArgsParse, String> {
    let Some(command) = args.next() else {
      return Err("missing subcommand; expected `zeromq` or `stdout`".to_string());
    };
    if matches!(command.as_str(), "-h" | "--help") {
      return Ok(ArgsParse::Help(Help::Root));
    }

    let mut parsed = Self {
      command: match command.as_str() {
        "zeromq" => Command::ZeroMq {
          endpoint: DEFAULT_ENDPOINT.to_string(),
        },
        "stdout" => Command::Stdout,
        _ => return Err(format!("unknown subcommand `{command}`; expected `zeromq` or `stdout`")),
      },
      codex_dir: None,
      pi_dir: None,
      poll_interval: DEFAULT_POLL_INTERVAL,
      new_file_replay: NewFileReplay::Messages(DEFAULT_REPLAY_MESSAGES),
    };
    let mut replay_option_seen = false;

    while let Some(arg) = args.next() {
      match arg.as_str() {
        "--bind" => match &mut parsed.command {
          Command::ZeroMq { endpoint } => *endpoint = next_value(&mut args, "--bind")?,
          Command::Stdout => return Err("`--bind` is only valid for the `zeromq` subcommand".to_string()),
        },
        "--codex-dir" => parsed.codex_dir = Some(PathBuf::from(next_value(&mut args, "--codex-dir")?)),
        "--pi-dir" => parsed.pi_dir = Some(PathBuf::from(next_value(&mut args, "--pi-dir")?)),
        "--poll-interval" => parsed.poll_interval = parse_duration(&next_value(&mut args, "--poll-interval")?)?,
        "--replay-all" => {
          set_replay_option(&mut replay_option_seen)?;
          parsed.new_file_replay = NewFileReplay::All;
        }
        replay if replay.starts_with("--replay=") => {
          set_replay_option(&mut replay_option_seen)?;
          let count = replay
            .strip_prefix("--replay=")
            .expect("replay prefix was checked")
            .parse()
            .map_err(|_| "`--replay` requires a non-negative integer".to_string())?;
          parsed.new_file_replay = NewFileReplay::Messages(count);
        }
        "-h" | "--help" => {
          let help = match parsed.command {
            Command::ZeroMq { .. } => Help::ZeroMq,
            Command::Stdout => Help::Stdout,
          };
          return Ok(ArgsParse::Help(help));
        }
        _ => return Err(format!("unknown argument `{arg}`; use --help for usage")),
      }
    }
    Ok(ArgsParse::Run(parsed))
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

fn set_replay_option(seen: &mut bool) -> Result<(), String> {
  if *seen {
    return Err("`--replay=<count>` and `--replay-all` are mutually exclusive".to_string());
  }
  *seen = true;
  Ok(())
}

fn parse_duration(value: &str) -> Result<Duration, String> {
  let (amount, unit) = if let Some(value) = value.strip_suffix("ms") {
    (value, "ms")
  } else if let Some(value) = value.strip_suffix('s') {
    (value, "s")
  } else if let Some(value) = value.strip_suffix('m') {
    (value, "m")
  } else {
    return Err(format!(
      "invalid duration `{value}`; expected a value such as 250ms, 2s, or 1m"
    ));
  };
  let amount = amount
    .parse::<u64>()
    .map_err(|_| format!("invalid duration `{value}`; expected a positive integer"))?;
  let duration = match unit {
    "ms" => Duration::from_millis(amount),
    "s" => Duration::from_secs(amount),
    "m" => Duration::from_secs(amount.saturating_mul(60)),
    _ => unreachable!(),
  };
  if duration.is_zero() {
    return Err("poll interval must be greater than zero".to_string());
  }
  Ok(duration)
}

fn resolve_root(explicit: &Option<PathBuf>, home: Option<&Path>, relative: &str) -> Result<PathBuf, String> {
  explicit
    .clone()
    .or_else(|| home.map(|home| home.join(relative)))
    .ok_or_else(|| format!("HOME is not set; pass an explicit directory for {relative}"))
}

fn print_help(help: Help) {
  match help {
    Help::Root => println!(
      "\
Follow Codex and Pi session files and emit normalized events.

Usage:
  tokn-session-relay <subcommand> [options]

Subcommands:
  zeromq  Publish two-frame ZeroMQ messages
  stdout  Write AgentEvent JSONL to stdout

Run `tokn-session-relay <subcommand> --help` for details."
    ),
    Help::ZeroMq => println!(
      "\
Publish normalized events over ZeroMQ.

Usage:
  tokn-session-relay zeromq [options]

Options:
  --bind <endpoint>           PUB endpoint (default: {DEFAULT_ENDPOINT})
  --codex-dir <path>          Codex session root (default: ~/.codex/sessions)
  --pi-dir <path>             Pi session root (default: ~/.pi/agent/sessions)
  --poll-interval <duration>  Fallback rescan interval, such as 250ms, 2s, or 1m (default: 2s)
  --replay=<count>            Messages replayed for a newly discovered file (default: 3)
  --replay-all                Replay all records for a newly discovered file
  -h, --help                  Show this help

Messages use two frames:
  1. topic: codex.<session_id> or pi.<session_id>
  2. JSON: normalized AgentEvent"
    ),
    Help::Stdout => println!(
      "\
Write normalized AgentEvent JSONL to stdout.

Usage:
  tokn-session-relay stdout [options]

Options:
  --codex-dir <path>          Codex session root (default: ~/.codex/sessions)
  --pi-dir <path>             Pi session root (default: ~/.pi/agent/sessions)
  --poll-interval <duration>  Fallback rescan interval, such as 250ms, 2s, or 1m (default: 2s)
  --replay=<count>            Messages replayed for a newly discovered file (default: 3)
  --replay-all                Replay all records for a newly discovered file
  -h, --help                  Show this help"
    ),
  }
}

#[cfg(test)]
mod tests {
  use std::io::{self, Write};
  use std::path::PathBuf;
  use std::time::Duration;

  use tokn_session_core::{AgentEvent, MessageEvent, Phase, Provider, Role};
  use tokn_session_relay::{NewFileReplay, RelayEvent};

  use super::{Args, ArgsParse, Command, write_jsonl_event};

  #[test]
  fn parses_zeromq_subcommand_and_shared_options() {
    let ArgsParse::Run(args) = Args::parse(
      [
        "zeromq",
        "--bind",
        "ipc:///tmp/relay.sock",
        "--replay=5",
        "--poll-interval",
        "250ms",
      ]
      .into_iter()
      .map(str::to_string),
    )
    .unwrap() else {
      panic!("expected runnable args");
    };
    assert_eq!(
      args.command,
      Command::ZeroMq {
        endpoint: "ipc:///tmp/relay.sock".to_string()
      }
    );
    assert_eq!(args.poll_interval, Duration::from_millis(250));
    assert_eq!(args.new_file_replay, NewFileReplay::Messages(5));
  }

  #[test]
  fn requires_a_subcommand() {
    assert!(Args::parse(std::iter::empty()).is_err());
    assert!(Args::parse(["--replay=3".to_string()].into_iter()).is_err());
  }

  #[test]
  fn rejects_zeromq_options_for_stdout() {
    let result = Args::parse(
      ["stdout", "--bind", "tcp://127.0.0.1:5556"]
        .into_iter()
        .map(str::to_string),
    );
    assert!(result.is_err());
  }

  #[test]
  fn parses_replay_all_and_rejects_conflicting_replay_options() {
    let ArgsParse::Run(args) = Args::parse(["stdout", "--replay-all"].into_iter().map(str::to_string)).unwrap() else {
      panic!("expected runnable args");
    };
    assert_eq!(args.new_file_replay, NewFileReplay::All);

    let result = Args::parse(["stdout", "--replay=3", "--replay-all"].into_iter().map(str::to_string));
    assert!(result.is_err());
  }

  #[test]
  fn writes_raw_agent_event_jsonl() {
    let event = RelayEvent {
      path: PathBuf::from("session.jsonl"),
      topic: "pi.session-1".to_string(),
      event: AgentEvent::Message(MessageEvent {
        provider: Provider::Pi,
        session_id: Some("session-1".to_string()),
        message_id: None,
        parent_id: None,
        role: Role::Assistant,
        phase: Phase::Finished,
        text: "done".to_string(),
        timestamp: None,
      }),
    };
    let mut output = RecordingWriter::default();
    write_jsonl_event(&mut output, &event).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.bytes).unwrap();
    assert_eq!(value["type"], "message");
    assert_eq!(value["text"], "done");
    assert!(value.get("topic").is_none());
    assert_eq!(output.flushes, 1);
  }

  #[derive(Default)]
  struct RecordingWriter {
    bytes: Vec<u8>,
    flushes: usize,
  }

  impl Write for RecordingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
      self.bytes.extend_from_slice(buffer);
      Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
      self.flushes += 1;
      Ok(())
    }
  }
}
