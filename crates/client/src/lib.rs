use std::path::PathBuf;
use std::process::{Command, Stdio};

use tokn_session_codex::CodexSessionSource;
use tokn_session_core::{LoadedSession, SessionRef};
use tokn_session_opencode::OpenCodeSessionSource;
use tokn_session_pi::PiSessionSource;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
  Pi,
  Codex,
  OpenCode,
}

pub struct AgentClient;

impl AgentClient {
  pub fn list_sessions(source: Source, session_dir: Option<PathBuf>) -> Result<Vec<SessionRef>, String> {
    session_source(source, session_dir)?.list_sessions()
  }

  pub fn load_session(source: Source, session_dir: Option<PathBuf>, session: &str) -> Result<LoadedSession, String> {
    session_source(source, session_dir)?.load_session(session)
  }

  pub fn create_session(request: CreateSessionRequest) -> Result<(), String> {
    let executor = request
      .executor
      .or_else(|| executor_from_env(request.source))
      .ok_or_else(|| executor_required_error(request.source))?;
    let mut command = executor_command(request.source, &executor, &request.prompt)?;
    if let Some(cwd) = request.cwd {
      command.current_dir(cwd);
    }

    let status = command
      .stdin(Stdio::inherit())
      .stdout(Stdio::inherit())
      .stderr(Stdio::inherit())
      .status()
      .map_err(|err| format!("failed to run create executor `{executor}`: {err}"))?;

    if status.success() {
      return Ok(());
    }

    Err(format!(
      "create executor `{executor}` exited with {}",
      status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string())
    ))
  }
}

pub struct CreateSessionRequest {
  pub source: Source,
  pub executor: Option<String>,
  pub cwd: Option<PathBuf>,
  pub prompt: String,
}

impl Source {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Pi => "pi",
      Self::Codex => "codex",
      Self::OpenCode => "opencode",
    }
  }

  fn create_args(self, prompt: &str) -> Vec<String> {
    match self {
      Self::Pi => vec![
        "--mode".to_string(),
        "json".to_string(),
        "--print".to_string(),
        prompt.to_string(),
      ],
      Self::Codex => vec!["exec".to_string(), "--json".to_string(), prompt.to_string()],
      Self::OpenCode => vec![
        "run".to_string(),
        "--format".to_string(),
        "json".to_string(),
        prompt.to_string(),
      ],
    }
  }
}

fn executor_from_env(source: Source) -> Option<String> {
  let source_key = format!("TOKN_SESSION_{}_EXECUTOR", source.as_str().to_ascii_uppercase());
  std::env::var(source_key)
    .ok()
    .filter(|value| !value.trim().is_empty())
    .or_else(|| std::env::var("TOKN_SESSION_EXECUTOR").ok())
    .filter(|value| !value.trim().is_empty())
}

fn executor_required_error(source: Source) -> String {
  format!(
    "create requires --executor or TOKN_SESSION_{}_EXECUTOR, for example: --executor \"tokn-gateway proxy {} --npx --\"",
    source.as_str().to_ascii_uppercase(),
    source.as_str()
  )
}

fn executor_command(source: Source, executor: &str, prompt: &str) -> Result<Command, String> {
  let mut parts = create_argv(source, executor, prompt)?;
  let program = parts.remove(0);
  let mut command = Command::new(program);
  command.args(parts);
  Ok(command)
}

fn create_argv(source: Source, executor: &str, prompt: &str) -> Result<Vec<String>, String> {
  let mut parts = split_command_line(executor)?;
  if parts.is_empty() {
    return Err("create executor cannot be empty".to_string());
  }

  let has_prompt_placeholder = parts.iter().any(|part| part == "{prompt}");
  if has_prompt_placeholder {
    for part in &mut parts {
      if part == "{prompt}" {
        *part = prompt.to_string();
      }
    }
  } else {
    parts.extend(source.create_args(prompt));
  }

  Ok(parts)
}

fn split_command_line(input: &str) -> Result<Vec<String>, String> {
  let mut args = Vec::new();
  let mut current = String::new();
  let mut chars = input.chars().peekable();
  let mut quote = None;

  while let Some(ch) = chars.next() {
    match (quote, ch) {
      (Some(q), ch) if ch == q => quote = None,
      (Some(_), '\\') => {
        if let Some(next) = chars.next() {
          current.push(next);
        } else {
          current.push('\\');
        }
      }
      (Some(_), ch) => current.push(ch),
      (None, '\'' | '"') => quote = Some(ch),
      (None, '\\') => {
        if let Some(next) = chars.next() {
          current.push(next);
        } else {
          current.push('\\');
        }
      }
      (None, ch) if ch.is_whitespace() => {
        if !current.is_empty() {
          args.push(std::mem::take(&mut current));
        }
      }
      (None, ch) => current.push(ch),
    }
  }

  if let Some(quote) = quote {
    return Err(format!("unterminated quote `{quote}` in create executor"));
  }

  if !current.is_empty() {
    args.push(current);
  }

  Ok(args)
}

#[cfg(test)]
mod tests {
  use super::{Source, create_argv, split_command_line};

  #[test]
  fn split_command_line_preserves_quoted_arguments() {
    let args = split_command_line(r#"tokn-gateway proxy opencode --npx -- run "{prompt}""#).unwrap();

    assert_eq!(
      args,
      vec!["tokn-gateway", "proxy", "opencode", "--npx", "--", "run", "{prompt}"]
    );
  }

  #[test]
  fn create_argv_appends_opencode_create_args_after_executor_launcher() {
    let args = create_argv(
      Source::OpenCode,
      "tokn-gateway proxy opencode --npx --",
      "create a todo app",
    )
    .unwrap();

    assert_eq!(
      args,
      vec![
        "tokn-gateway",
        "proxy",
        "opencode",
        "--npx",
        "--",
        "run",
        "--format",
        "json",
        "create a todo app"
      ]
    );
  }

  #[test]
  fn create_argv_uses_placeholder_as_advanced_full_command_override() {
    let args = create_argv(Source::OpenCode, "custom-agent --message {prompt}", "create a todo app").unwrap();

    assert_eq!(args, vec!["custom-agent", "--message", "create a todo app"]);
  }
}

enum SessionSourceClient {
  Codex(CodexSessionSource),
  OpenCode(OpenCodeSessionSource),
  Pi(PiSessionSource),
}

impl SessionSourceClient {
  fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    match self {
      Self::Codex(source) => source.list_sessions(),
      Self::OpenCode(source) => source.list_sessions(),
      Self::Pi(source) => source.list_sessions(),
    }
  }

  fn load_session(&self, session: &str) -> Result<LoadedSession, String> {
    match self {
      Self::Codex(source) => source.load_session(session),
      Self::OpenCode(source) => source.load_session(session),
      Self::Pi(source) => source.load_session(session),
    }
  }
}

fn session_source(source: Source, session_dir: Option<PathBuf>) -> Result<SessionSourceClient, String> {
  match source {
    Source::Pi => Ok(SessionSourceClient::Pi(PiSessionSource::new(session_dir))),
    Source::Codex => Ok(SessionSourceClient::Codex(CodexSessionSource::new(session_dir))),
    Source::OpenCode => Ok(SessionSourceClient::OpenCode(OpenCodeSessionSource::new(session_dir))),
  }
}
