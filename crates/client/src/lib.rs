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
    run_print_session(PrintSessionRequest {
      source: request.source,
      executor: request.executor,
      cwd: request.cwd,
      action: PrintAction::Create { prompt: request.prompt },
    })
  }

  pub fn append_session(request: AppendSessionRequest) -> Result<(), String> {
    run_print_session(PrintSessionRequest {
      source: request.source,
      executor: request.executor,
      cwd: request.cwd,
      action: request.action.into(),
    })
  }
}

pub struct CreateSessionRequest {
  pub source: Source,
  pub executor: Option<String>,
  pub cwd: Option<PathBuf>,
  pub prompt: String,
}

pub struct AppendSessionRequest {
  pub source: Source,
  pub executor: Option<String>,
  pub cwd: Option<PathBuf>,
  pub action: AppendAction,
}

pub enum AppendAction {
  Continue { prompt: String },
  Session { session_id: String, prompt: String },
}

struct PrintSessionRequest {
  source: Source,
  executor: Option<String>,
  cwd: Option<PathBuf>,
  action: PrintAction,
}

enum PrintAction {
  Create { prompt: String },
  Continue { prompt: String },
  Session { session_id: String, prompt: String },
}

impl From<AppendAction> for PrintAction {
  fn from(action: AppendAction) -> Self {
    match action {
      AppendAction::Continue { prompt } => Self::Continue { prompt },
      AppendAction::Session { session_id, prompt } => Self::Session { session_id, prompt },
    }
  }
}

impl Source {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Pi => "pi",
      Self::Codex => "codex",
      Self::OpenCode => "opencode",
    }
  }

  fn print_args(self, action: &PrintAction) -> Vec<String> {
    match self {
      Self::Pi => pi_print_args(action),
      Self::Codex => codex_print_args(action),
      Self::OpenCode => opencode_print_args(action),
    }
  }
}

fn run_print_session(request: PrintSessionRequest) -> Result<(), String> {
  let executor = request
    .executor
    .or_else(|| executor_from_env(request.source))
    .ok_or_else(|| executor_required_error(request.source))?;
  let mut command = executor_command(request.source, &executor, &request.action)?;
  if let Some(cwd) = request.cwd {
    command.current_dir(cwd);
  }

  let status = command
    .stdin(Stdio::inherit())
    .stdout(Stdio::inherit())
    .stderr(Stdio::inherit())
    .status()
    .map_err(|err| format!("failed to run executor `{executor}`: {err}"))?;

  if status.success() {
    return Ok(());
  }

  Err(format!(
    "executor `{executor}` exited with {}",
    status
      .code()
      .map(|code| code.to_string())
      .unwrap_or_else(|| "signal".to_string())
  ))
}

fn opencode_print_args(action: &PrintAction) -> Vec<String> {
  let mut args = vec!["run".to_string(), "--format".to_string(), "json".to_string()];
  match action {
    PrintAction::Create { prompt } => args.push(prompt.clone()),
    PrintAction::Continue { prompt } => {
      args.push("--continue".to_string());
      args.push(prompt.clone());
    }
    PrintAction::Session { session_id, prompt } => {
      args.push("--session".to_string());
      args.push(session_id.clone());
      args.push(prompt.clone());
    }
  }
  args
}

fn codex_print_args(action: &PrintAction) -> Vec<String> {
  match action {
    PrintAction::Create { prompt } => vec!["exec".to_string(), "--json".to_string(), prompt.clone()],
    PrintAction::Continue { prompt } => vec![
      "exec".to_string(),
      "--json".to_string(),
      "resume".to_string(),
      "--last".to_string(),
      prompt.clone(),
    ],
    PrintAction::Session { session_id, prompt } => vec![
      "exec".to_string(),
      "--json".to_string(),
      "resume".to_string(),
      session_id.clone(),
      prompt.clone(),
    ],
  }
}

fn pi_print_args(action: &PrintAction) -> Vec<String> {
  let mut args = vec!["--mode".to_string(), "json".to_string()];
  match action {
    PrintAction::Create { prompt } => {
      args.push("--print".to_string());
      args.push(prompt.clone());
    }
    PrintAction::Continue { prompt } => {
      args.push("--continue".to_string());
      args.push(prompt.clone());
    }
    PrintAction::Session { session_id, prompt } => {
      args.push("--session".to_string());
      args.push(session_id.clone());
      args.push(prompt.clone());
    }
  }
  args
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

fn executor_command(source: Source, executor: &str, action: &PrintAction) -> Result<Command, String> {
  let mut parts = print_argv(source, executor, action)?;
  let program = parts.remove(0);
  let mut command = Command::new(program);
  command.args(parts);
  Ok(command)
}

fn print_argv(source: Source, executor: &str, action: &PrintAction) -> Result<Vec<String>, String> {
  let mut parts = split_command_line(executor)?;
  if parts.is_empty() {
    return Err("executor cannot be empty".to_string());
  }

  let has_prompt_placeholder = parts.iter().any(|part| part == "{prompt}");
  if has_prompt_placeholder {
    let prompt = action_prompt(action);
    for part in &mut parts {
      if part == "{prompt}" {
        *part = prompt.to_string();
      }
    }
  } else {
    parts.extend(source.print_args(action));
  }

  Ok(parts)
}

fn action_prompt(action: &PrintAction) -> &str {
  match action {
    PrintAction::Create { prompt } => prompt,
    PrintAction::Continue { prompt } => prompt,
    PrintAction::Session { prompt, .. } => prompt,
  }
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
  use super::{PrintAction, Source, print_argv, split_command_line};

  #[test]
  fn split_command_line_preserves_quoted_arguments() {
    let args = split_command_line(r#"tokn-gateway proxy opencode --npx -- run "{prompt}""#).unwrap();

    assert_eq!(
      args,
      vec!["tokn-gateway", "proxy", "opencode", "--npx", "--", "run", "{prompt}"]
    );
  }

  #[test]
  fn print_argv_appends_opencode_create_args_after_executor_launcher() {
    let args = print_argv(
      Source::OpenCode,
      "tokn-gateway proxy opencode --npx --",
      &PrintAction::Create {
        prompt: "create a todo app".to_string(),
      },
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
  fn print_argv_appends_opencode_session_args_after_executor_launcher() {
    let args = print_argv(
      Source::OpenCode,
      "tokn-gateway proxy opencode --npx --",
      &PrintAction::Session {
        session_id: "ses_123".to_string(),
        prompt: "next turn".to_string(),
      },
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
        "--session",
        "ses_123",
        "next turn"
      ]
    );
  }

  #[test]
  fn print_argv_appends_opencode_continue_args_after_executor_launcher() {
    let args = print_argv(
      Source::OpenCode,
      "tokn-gateway proxy opencode --npx --",
      &PrintAction::Continue {
        prompt: "next turn".to_string(),
      },
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
        "--continue",
        "next turn"
      ]
    );
  }

  #[test]
  fn print_argv_uses_placeholder_as_advanced_full_command_override() {
    let args = print_argv(
      Source::OpenCode,
      "custom-agent --message {prompt}",
      &PrintAction::Create {
        prompt: "create a todo app".to_string(),
      },
    )
    .unwrap();

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
