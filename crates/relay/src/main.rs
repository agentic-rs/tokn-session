use std::collections::HashSet;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokn_session_core::Provider;
use tokn_session_relay::{
  DEFAULT_POLL_INTERVAL, DEFAULT_REPLAY_MESSAGES, NewFileReplay, ProviderRoot, RelayConfig, RelayEvent, SessionRelay,
  TailUpdate, ZmqPublisher,
};
use tokn_session_render::{render_event_pretty, render_event_summary};

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
    Command::Stdout { format, color } => {
      eprintln!("following Codex/Pi session events on stdout ({})", format.name());
      Output::Stdout {
        writer: BufWriter::new(std::io::stdout()),
        format,
        color,
        seen_sessions: HashSet::new(),
      }
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
  Stdout {
    writer: BufWriter<std::io::Stdout>,
    format: StdoutFormat,
    color: bool,
    seen_sessions: HashSet<String>,
  },
}

impl Output {
  async fn write_update(&mut self, update: TailUpdate) {
    for warning in update.warnings {
      eprintln!("warning: {warning}");
    }
    for event in update.events {
      let result = match self {
        Self::ZeroMq(publisher) => publisher.publish(&event).await,
        Self::Stdout {
          writer,
          format,
          color,
          seen_sessions,
        } => {
          let show_context = *format == StdoutFormat::Pretty && seen_sessions.insert(event.topic.clone());
          write_stdout_event(writer, &event, *format, *color, show_context)
        }
      };
      if let Err(err) = result {
        eprintln!("warning: {err}");
      }
    }
  }
}

fn write_jsonl_event(writer: &mut impl Write, event: &RelayEvent) -> Result<(), String> {
  serde_json::to_writer(&mut *writer, event).map_err(|err| format!("failed to serialize relay event: {err}"))?;
  writer
    .write_all(b"\n")
    .and_then(|_| writer.flush())
    .map_err(|err| format!("failed to write relay event: {err}"))
}

fn write_stdout_event(
  writer: &mut impl Write,
  event: &RelayEvent,
  format: StdoutFormat,
  color: bool,
  show_context: bool,
) -> Result<(), String> {
  if format == StdoutFormat::Json {
    return write_jsonl_event(writer, event);
  }

  let rendered = match format {
    StdoutFormat::Pretty => {
      let pretty = render_event_pretty(&event.event);
      if pretty.is_empty() {
        format!("{}\n\n", render_event_summary(&event.event))
      } else {
        pretty
      }
    }
    StdoutFormat::Summary => format!("{}\n", render_event_summary(&event.event)),
    StdoutFormat::Json => unreachable!(),
  };
  let mut output = String::new();
  if show_context {
    output.push_str(&render_session_context(event, color));
  }
  output.push_str(&prefix_human_output(event, &rendered, color));
  writer
    .write_all(output.as_bytes())
    .and_then(|_| writer.flush())
    .map_err(|err| format!("failed to write relay event: {err}"))
}

fn prefix_human_output(event: &RelayEvent, rendered: &str, color: bool) -> String {
  let (first_line, remainder) = rendered.split_once('\n').unwrap_or((rendered, ""));
  let mut output = String::new();
  let prefix = human_event_prefix(event);
  if color {
    output.push_str(event_color(&event.event));
  }
  output.push_str(&prefix);
  if color {
    output.push_str(ANSI_RESET);
  }
  if !first_line.is_empty() {
    output.push(' ');
    if color {
      output.push_str(event_color(&event.event));
    }
    output.push_str(first_line);
    if color {
      output.push_str(ANSI_RESET);
    }
  }
  output.push('\n');
  output.push_str(remainder);
  output
}

fn human_event_prefix(event: &RelayEvent) -> String {
  let mut parts = Vec::new();
  if let Some(timestamp) = event_timestamp(&event.event).and_then(display_timestamp) {
    parts.push(timestamp);
  }
  if let Some(project) = event
    .session
    .project
    .as_ref()
    .and_then(|project| project.name.as_deref())
  {
    parts.push(project.to_string());
  }
  parts.push(format!(
    "{}/{}",
    provider_name(event.session.provider),
    abbreviate_id(&event.session.session_id)
  ));
  if let Some(message_id) = event_message_id(&event.event) {
    parts.push(format!("#{}", abbreviate_id(message_id)));
  }
  if let Some(parent_id) = event_parent_id(&event.event) {
    parts.push(format!("←#{}", abbreviate_id(parent_id)));
  }
  parts.join(" ")
}

fn render_session_context(event: &RelayEvent, color: bool) -> String {
  let context = &event.session;
  let mut output = String::new();
  if color {
    output.push_str(ANSI_BLUE);
  }
  output.push_str("session ");
  output.push_str(provider_name(context.provider));
  output.push('/');
  output.push_str(&context.session_id);
  if color {
    output.push_str(ANSI_RESET);
  }
  output.push('\n');

  append_context_line(&mut output, "title", context.title.as_deref(), color);
  append_context_line(&mut output, "parent", context.parent_session_id.as_deref(), color);
  append_context_line(&mut output, "started", context.started_at.as_deref(), color);

  if let Some(project) = &context.project {
    append_context_line(&mut output, "project", project.name.as_deref(), color);
    append_context_line(&mut output, "folder", project.folder.as_deref(), color);
    if context.cwd.as_deref() != project.folder.as_deref() {
      append_context_line(&mut output, "cwd", context.cwd.as_deref(), color);
    }
    append_context_line(&mut output, "repository", project.repository_url.as_deref(), color);
    append_context_line(&mut output, "branch", project.branch.as_deref(), color);
    append_context_line(&mut output, "commit", project.commit_hash.as_deref(), color);
  } else {
    append_context_line(&mut output, "cwd", context.cwd.as_deref(), color);
  }
  output.push('\n');
  output
}

fn append_context_line(output: &mut String, label: &str, value: Option<&str>, color: bool) {
  let Some(value) = value else {
    return;
  };
  output.push_str("  ");
  if color {
    output.push_str(ANSI_DIM);
  }
  output.push_str(label);
  if color {
    output.push_str(ANSI_RESET);
  }
  output.push(' ');
  output.push_str(value);
  output.push('\n');
}

fn provider_name(provider: Provider) -> &'static str {
  match provider {
    Provider::Pi => "pi",
    Provider::Codex => "codex",
    Provider::OpenCode => "opencode",
  }
}

fn display_timestamp(timestamp: &str) -> Option<String> {
  if timestamp.is_empty() {
    return None;
  }
  if let Some(time) = timestamp.split_once('T').map(|(_, time)| time) {
    return Some(time.chars().take(8).collect());
  }
  Some(timestamp.to_string())
}

fn abbreviate_id(id: &str) -> String {
  const DISPLAY_CHARS: usize = 8;

  if id.chars().count() <= DISPLAY_CHARS + 2 {
    return id.to_string();
  }
  format!("{}…", id.chars().take(DISPLAY_CHARS).collect::<String>())
}

fn event_timestamp(event: &tokn_session_core::AgentEvent) -> Option<&str> {
  use tokn_session_core::AgentEvent;

  match event {
    AgentEvent::SessionStarted(event) => event.timestamp.as_deref(),
    AgentEvent::ProviderChanged(event) => event.timestamp.as_deref(),
    AgentEvent::SessionSettingsApplied(event) => event.timestamp.as_deref(),
    AgentEvent::Message(event) => event.timestamp.as_deref(),
    AgentEvent::Reasoning(event) => event.timestamp.as_deref(),
    AgentEvent::GoalUpdated(event) => event.timestamp.as_deref(),
    AgentEvent::AgentActivity(event) => event.timestamp.as_deref(),
    AgentEvent::ToolCall(event) => event.timestamp.as_deref(),
    AgentEvent::Error(event) => event.timestamp.as_deref(),
    AgentEvent::Unknown(event) => event.timestamp.as_deref(),
  }
}

fn event_message_id(event: &tokn_session_core::AgentEvent) -> Option<&str> {
  use tokn_session_core::AgentEvent;

  match event {
    AgentEvent::Message(event) => event.message_id.as_deref(),
    AgentEvent::Reasoning(event) => event.message_id.as_deref(),
    AgentEvent::ToolCall(event) => event.message_id.as_deref(),
    _ => None,
  }
}

fn event_parent_id(event: &tokn_session_core::AgentEvent) -> Option<&str> {
  use tokn_session_core::AgentEvent;

  match event {
    AgentEvent::Message(event) => event.parent_id.as_deref(),
    AgentEvent::Reasoning(event) => event.parent_id.as_deref(),
    AgentEvent::ToolCall(event) => event.parent_id.as_deref(),
    _ => None,
  }
}

fn event_color(event: &tokn_session_core::AgentEvent) -> &'static str {
  use tokn_session_core::{AgentEvent, Role};

  match event {
    AgentEvent::SessionStarted(_)
    | AgentEvent::ProviderChanged(_)
    | AgentEvent::SessionSettingsApplied(_)
    | AgentEvent::GoalUpdated(_)
    | AgentEvent::AgentActivity(_) => ANSI_BLUE,
    AgentEvent::Message(event) => match event.role {
      Role::User => ANSI_CYAN,
      Role::Assistant => ANSI_GREEN,
      Role::System => ANSI_BLUE,
      Role::Tool => ANSI_YELLOW,
      Role::Unknown => ANSI_DIM,
    },
    AgentEvent::Reasoning(_) => ANSI_MAGENTA,
    AgentEvent::ToolCall(event) if event.is_error == Some(true) => ANSI_BOLD_RED,
    AgentEvent::ToolCall(_) => ANSI_YELLOW,
    AgentEvent::Error(_) => ANSI_BOLD_RED,
    AgentEvent::Unknown(_) => ANSI_DIM,
  }
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BLUE: &str = "\x1b[34m";
const ANSI_MAGENTA: &str = "\x1b[35m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_DIM: &str = "\x1b[2m";

#[derive(Debug, Eq, PartialEq)]
enum Command {
  ZeroMq { endpoint: String },
  Stdout { format: StdoutFormat, color: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StdoutFormat {
  Pretty,
  Summary,
  Json,
}

impl StdoutFormat {
  fn parse(value: &str) -> Result<Self, String> {
    match value {
      "pretty" => Ok(Self::Pretty),
      "summary" => Ok(Self::Summary),
      "json" => Ok(Self::Json),
      _ => Err(format!(
        "unknown stdout format `{value}`; expected `pretty`, `summary`, or `json`"
      )),
    }
  }

  fn name(self) -> &'static str {
    match self {
      Self::Pretty => "pretty",
      Self::Summary => "summary",
      Self::Json => "json",
    }
  }
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
        "stdout" => Command::Stdout {
          format: StdoutFormat::Summary,
          color: false,
        },
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
          Command::Stdout { .. } => return Err("`--bind` is only valid for the `zeromq` subcommand".to_string()),
        },
        "--format" => match &mut parsed.command {
          Command::Stdout { format, .. } => *format = StdoutFormat::parse(&next_value(&mut args, "--format")?)?,
          Command::ZeroMq { .. } => return Err("`--format` is only valid for the `stdout` subcommand".to_string()),
        },
        "--color" => match &mut parsed.command {
          Command::Stdout { color, .. } => *color = true,
          Command::ZeroMq { .. } => return Err("`--color` is only valid for the `stdout` subcommand".to_string()),
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
            Command::Stdout { .. } => Help::Stdout,
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
  stdout  Write formatted AgentEvent output to stdout

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
  2. JSON: RelayEvent envelope with session context and normalized AgentEvent"
    ),
    Help::Stdout => println!(
      "\
Write relay events with session context to stdout.

Usage:
  tokn-session-relay stdout [options]

Options:
  --format <format>           Output format: pretty, summary, or json (default: summary)
  --color                     Add ANSI colors to pretty or summary output
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

  use tokn_session_core::{AgentEvent, MessageEvent, Phase, Provider, Role, SessionStarted};
  use tokn_session_relay::{NewFileReplay, ProjectContext, RelayEvent, SessionContext};

  use super::{Args, ArgsParse, Command, StdoutFormat, write_jsonl_event, write_stdout_event};

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
  fn rejects_options_for_the_wrong_subcommand() {
    let result = Args::parse(
      ["stdout", "--bind", "tcp://127.0.0.1:5556"]
        .into_iter()
        .map(str::to_string),
    );
    assert!(result.is_err());
    let result = Args::parse(["zeromq", "--format", "pretty"].into_iter().map(str::to_string));
    assert!(result.is_err());
    let result = Args::parse(["zeromq", "--color"].into_iter().map(str::to_string));
    assert!(result.is_err());
  }

  #[test]
  fn parses_stdout_format_and_color() {
    let ArgsParse::Run(defaults) = Args::parse(["stdout"].into_iter().map(str::to_string)).unwrap() else {
      panic!("expected runnable args");
    };
    assert_eq!(
      defaults.command,
      Command::Stdout {
        format: StdoutFormat::Summary,
        color: false,
      }
    );

    let ArgsParse::Run(args) = Args::parse(
      ["stdout", "--format", "pretty", "--color"]
        .into_iter()
        .map(str::to_string),
    )
    .unwrap() else {
      panic!("expected runnable args");
    };
    assert_eq!(
      args.command,
      Command::Stdout {
        format: StdoutFormat::Pretty,
        color: true,
      }
    );

    let ArgsParse::Run(json) = Args::parse(
      ["stdout", "--format", "json", "--color"]
        .into_iter()
        .map(str::to_string),
    )
    .unwrap() else {
      panic!("expected runnable args");
    };
    assert_eq!(
      json.command,
      Command::Stdout {
        format: StdoutFormat::Json,
        color: true,
      }
    );
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
  fn writes_relay_event_jsonl() {
    let event = message_event();
    let mut output = RecordingWriter::default();
    write_jsonl_event(&mut output, &event).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.bytes).unwrap();
    assert_eq!(value["topic"], "pi.session-1");
    assert_eq!(value["session"]["project"]["name"], "project");
    assert_eq!(value["event"]["type"], "message");
    assert_eq!(value["event"]["text"], "done");
    assert_eq!(output.flushes, 1);
  }

  #[test]
  fn renders_summary_pretty_and_colorless_json() {
    let event = message_event();

    let mut summary = RecordingWriter::default();
    write_stdout_event(&mut summary, &event, StdoutFormat::Summary, false, false).unwrap();
    assert_eq!(
      String::from_utf8(summary.bytes).unwrap(),
      "00:00:01 project pi/session-1 #message-1 ←#parent-1 assistant done\n"
    );

    let mut pretty = RecordingWriter::default();
    write_stdout_event(&mut pretty, &event, StdoutFormat::Pretty, false, true).unwrap();
    assert_eq!(
      String::from_utf8(pretty.bytes).unwrap(),
      concat!(
        "session pi/session-1\n",
        "  started 2026-01-01T00:00:00Z\n",
        "  project project\n",
        "  folder /tmp/project\n",
        "\n",
        "00:00:01 project pi/session-1 #message-1 ←#parent-1 assistant\n",
        "  done\n",
        "\n",
      )
    );

    let mut colored = RecordingWriter::default();
    write_stdout_event(&mut colored, &event, StdoutFormat::Summary, true, false).unwrap();
    assert!(String::from_utf8(colored.bytes).unwrap().contains("\u{1b}["));

    let mut json = RecordingWriter::default();
    write_stdout_event(&mut json, &event, StdoutFormat::Json, true, false).unwrap();
    assert!(!String::from_utf8(json.bytes).unwrap().contains("\u{1b}["));
  }

  #[test]
  fn pretty_keeps_session_start_events_visible() {
    let event = RelayEvent {
      path: PathBuf::from("session.jsonl"),
      topic: "pi.session-1".to_string(),
      session: session_context(),
      event: AgentEvent::SessionStarted(SessionStarted {
        provider: Provider::Pi,
        session_id: "session-1".to_string(),
        cwd: None,
        timestamp: None,
      }),
    };
    let mut output = RecordingWriter::default();
    write_stdout_event(&mut output, &event, StdoutFormat::Pretty, false, false).unwrap();
    assert_eq!(
      String::from_utf8(output.bytes).unwrap(),
      "project pi/session-1 session started session-1\n\n"
    );
  }

  fn message_event() -> RelayEvent {
    RelayEvent {
      path: PathBuf::from("session.jsonl"),
      topic: "pi.session-1".to_string(),
      session: session_context(),
      event: AgentEvent::Message(MessageEvent {
        provider: Provider::Pi,
        session_id: Some("session-1".to_string()),
        message_id: Some("message-1".to_string()),
        parent_id: Some("parent-1".to_string()),
        role: Role::Assistant,
        phase: Phase::Finished,
        text: "done".to_string(),
        timestamp: Some("2026-01-01T00:00:01Z".to_string()),
      }),
    }
  }

  fn session_context() -> SessionContext {
    SessionContext {
      provider: Provider::Pi,
      session_id: "session-1".to_string(),
      parent_session_id: None,
      title: None,
      cwd: Some("/tmp/project".to_string()),
      started_at: Some("2026-01-01T00:00:00Z".to_string()),
      project: Some(ProjectContext {
        id: None,
        name: Some("project".to_string()),
        folder: Some("/tmp/project".to_string()),
        repository_url: None,
        branch: None,
        commit_hash: None,
      }),
    }
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
