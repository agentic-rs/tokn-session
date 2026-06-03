use std::path::PathBuf;

#[derive(Debug)]
pub struct Cli {
  pub command: Command,
}

#[derive(Debug)]
pub enum Command {
  Sessions(SessionsCommand),
}

#[derive(Debug)]
pub enum SessionsCommand {
  List {
    source: Source,
    session_dir: Option<PathBuf>,
    limit: usize,
  },
  Show {
    source: Source,
    session: String,
    format: Format,
    session_dir: Option<PathBuf>,
  },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
  Pi,
  Codex,
  OpenCode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
  Pretty,
  Jsonl,
}

pub fn parse(args: Vec<String>) -> Result<Cli, String> {
  if args.is_empty() {
    return Err(help());
  }

  match args[0].as_str() {
    "sessions" => parse_sessions(&args[1..]),
    "--help" | "-h" | "help" => Err(help()),
    other => Err(format!("unknown command `{other}`\n\n{}", help())),
  }
}

fn parse_sessions(args: &[String]) -> Result<Cli, String> {
  if args.is_empty() {
    return Err(sessions_help());
  }

  match args[0].as_str() {
    "list" => {
      let Options {
        source,
        format: _,
        session_dir,
        limit,
        positionals,
      } = parse_options(&args[1..])?;
      if !positionals.is_empty() {
        return Err("sessions list does not accept positional arguments".to_string());
      }
      Ok(Cli {
        command: Command::Sessions(SessionsCommand::List {
          source,
          session_dir,
          limit,
        }),
      })
    }
    "show" => {
      let Options {
        source,
        format,
        session_dir,
        limit: _,
        mut positionals,
      } = parse_options(&args[1..])?;
      if positionals.len() != 1 {
        return Err("sessions show requires exactly one session id or path".to_string());
      }
      Ok(Cli {
        command: Command::Sessions(SessionsCommand::Show {
          source,
          session: positionals.remove(0),
          format,
          session_dir,
        }),
      })
    }
    "--help" | "-h" | "help" => Err(sessions_help()),
    other => Err(format!("unknown sessions command `{other}`\n\n{}", sessions_help())),
  }
}

struct Options {
  source: Source,
  format: Format,
  session_dir: Option<PathBuf>,
  limit: usize,
  positionals: Vec<String>,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
  let mut source = Source::Pi;
  let mut format = Format::Pretty;
  let mut session_dir = None;
  let mut limit = 20;
  let mut positionals = Vec::new();
  let mut index = 0;

  while index < args.len() {
    match args[index].as_str() {
      "--source" => {
        index += 1;
        let value = args.get(index).ok_or_else(|| "--source requires a value".to_string())?;
        source = parse_source(value)?;
      }
      "--format" => {
        index += 1;
        let value = args.get(index).ok_or_else(|| "--format requires a value".to_string())?;
        format = parse_format(value)?;
      }
      "--session-dir" => {
        index += 1;
        let value = args
          .get(index)
          .ok_or_else(|| "--session-dir requires a value".to_string())?;
        session_dir = Some(PathBuf::from(value));
      }
      "--limit" => {
        index += 1;
        let value = args.get(index).ok_or_else(|| "--limit requires a value".to_string())?;
        limit = value
          .parse::<usize>()
          .map_err(|_| format!("invalid --limit value `{value}`"))?;
      }
      "--help" | "-h" => return Err(sessions_help()),
      value if value.starts_with('-') => return Err(format!("unknown option `{value}`")),
      value => positionals.push(value.to_string()),
    }
    index += 1;
  }

  Ok(Options {
    source,
    format,
    session_dir,
    limit,
    positionals,
  })
}

fn parse_source(value: &str) -> Result<Source, String> {
  match value {
    "pi" => Ok(Source::Pi),
    "codex" => Ok(Source::Codex),
    "opencode" => Ok(Source::OpenCode),
    _ => Err(format!("unknown source `{value}`")),
  }
}

fn parse_format(value: &str) -> Result<Format, String> {
  match value {
    "pretty" => Ok(Format::Pretty),
    "jsonl" => Ok(Format::Jsonl),
    _ => Err(format!("unknown format `{value}`")),
  }
}

fn help() -> String {
  "usage:
  tokn-agent sessions list [--source pi] [--session-dir <dir>]
  tokn-agent sessions list [--source pi] [--limit <n>]
  tokn-agent sessions show [--source pi] [--format pretty|jsonl] [--session-dir <dir>] <session-id-or-path>"
    .to_string()
}

fn sessions_help() -> String {
  "usage:
  tokn-agent sessions list [--source pi] [--session-dir <dir>]
  tokn-agent sessions list [--source pi] [--limit <n>]
  tokn-agent sessions show [--source pi] [--format pretty|jsonl] [--session-dir <dir>] <session-id-or-path>"
    .to_string()
}
