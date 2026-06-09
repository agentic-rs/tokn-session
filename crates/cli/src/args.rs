use std::path::PathBuf;

use tokn_session_client::Source;

#[derive(Debug)]
pub struct Cli {
  pub command: Command,
}

#[derive(Debug)]
pub enum Command {
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
  Browse {
    source: Source,
    session: Option<String>,
    session_dir: Option<PathBuf>,
  },
  Create {
    source: Source,
    prompt: String,
    executor: Option<String>,
    cwd: Option<PathBuf>,
  },
  Append {
    source: Source,
    target: AppendTarget,
    prompt: String,
    executor: Option<String>,
    cwd: Option<PathBuf>,
  },
}

#[derive(Debug)]
pub enum AppendTarget {
  Continue,
  Session(String),
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
    "list" => {
      let Options {
        source,
        format: _,
        session_dir,
        limit,
        executor: _,
        cwd: _,
        append_target,
        positionals,
      } = parse_options(&args[1..])?;
      reject_append_target("list", append_target)?;
      if !positionals.is_empty() {
        return Err("list does not accept positional arguments".to_string());
      }
      Ok(Cli {
        command: Command::List {
          source,
          session_dir,
          limit,
        },
      })
    }
    "show" => {
      let Options {
        source,
        format,
        session_dir,
        limit: _,
        executor: _,
        cwd: _,
        append_target,
        mut positionals,
      } = parse_options(&args[1..])?;
      reject_append_target("show", append_target)?;
      if positionals.len() != 1 {
        return Err("show requires exactly one session id or path".to_string());
      }
      Ok(Cli {
        command: Command::Show {
          source,
          session: positionals.remove(0),
          format,
          session_dir,
        },
      })
    }
    "browse" => {
      let Options {
        source,
        format: _,
        session_dir,
        limit: _,
        executor: _,
        cwd: _,
        append_target,
        mut positionals,
      } = parse_options(&args[1..])?;
      reject_append_target("browse", append_target)?;
      if positionals.len() > 1 {
        return Err("browse accepts at most one session id or path".to_string());
      }
      Ok(Cli {
        command: Command::Browse {
          source,
          session: positionals.pop(),
          session_dir,
        },
      })
    }
    "create" => {
      let Options {
        source,
        format: _,
        session_dir: _,
        limit: _,
        executor,
        cwd,
        append_target,
        mut positionals,
      } = parse_options(&args[1..])?;
      reject_append_target("create", append_target)?;
      if positionals.len() != 1 {
        return Err("create requires exactly one prompt".to_string());
      }
      Ok(Cli {
        command: Command::Create {
          source,
          prompt: positionals.remove(0),
          executor,
          cwd,
        },
      })
    }
    "append" => {
      let Options {
        source,
        format: _,
        session_dir: _,
        limit: _,
        executor,
        cwd,
        append_target,
        mut positionals,
      } = parse_options(&args[1..])?;
      if positionals.len() != 1 {
        return Err("append requires exactly one prompt".to_string());
      }
      let target = append_target.ok_or_else(|| "append requires --continue or --session <id>".to_string())?;
      Ok(Cli {
        command: Command::Append {
          source,
          target,
          prompt: positionals.remove(0),
          executor,
          cwd,
        },
      })
    }
    "--help" | "-h" | "help" => Err(help()),
    other => Err(format!("unknown command `{other}`\n\n{}", help())),
  }
}

struct Options {
  source: Source,
  format: Format,
  session_dir: Option<PathBuf>,
  limit: usize,
  executor: Option<String>,
  cwd: Option<PathBuf>,
  append_target: Option<AppendTarget>,
  positionals: Vec<String>,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
  let mut source = Source::Pi;
  let mut format = Format::Pretty;
  let mut session_dir = None;
  let mut limit = 20;
  let mut executor = None;
  let mut cwd = None;
  let mut append_target = None;
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
      "--executor" => {
        index += 1;
        let value = args
          .get(index)
          .ok_or_else(|| "--executor requires a value".to_string())?;
        executor = Some(value.to_string());
      }
      "--cwd" => {
        index += 1;
        let value = args.get(index).ok_or_else(|| "--cwd requires a value".to_string())?;
        cwd = Some(PathBuf::from(value));
      }
      "--continue" => {
        if append_target.is_some() {
          return Err("append accepts only one of --continue or --session <id>".to_string());
        }
        append_target = Some(AppendTarget::Continue);
      }
      "--session" => {
        if append_target.is_some() {
          return Err("append accepts only one of --continue or --session <id>".to_string());
        }
        index += 1;
        let value = args
          .get(index)
          .ok_or_else(|| "--session requires a value".to_string())?;
        append_target = Some(AppendTarget::Session(value.to_string()));
      }
      "--help" | "-h" => return Err(help()),
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
    executor,
    cwd,
    append_target,
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

fn reject_append_target(command: &str, target: Option<AppendTarget>) -> Result<(), String> {
  if target.is_some() {
    return Err(format!(
      "--continue and --session are only valid for append, not {command}"
    ));
  }
  Ok(())
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
  tokn-session list [--source pi|codex|opencode] [--session-dir <dir>]
  tokn-session list [--source pi|codex|opencode] [--limit <n>]
  tokn-session show [--source pi|codex|opencode] [--format pretty|jsonl] [--session-dir <dir>] <session-id-or-path>
  tokn-session browse [--source pi|codex|opencode] [--session-dir <dir>] [session-id-or-path]
  tokn-session create [--source pi|codex|opencode] [--executor <command>] [--cwd <dir>] <prompt>
  tokn-session append [--source pi|codex|opencode] [--executor <command>] [--cwd <dir>] (--continue|--session <id>) <prompt>"
    .to_string()
}
