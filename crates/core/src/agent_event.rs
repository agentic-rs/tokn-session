use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
  SessionStarted(SessionStarted),
  ProviderChanged(ProviderChanged),
  Message(MessageEvent),
  Reasoning(ReasoningEvent),
  GoalUpdated(GoalUpdated),
  ToolCall(ToolCallEvent),
  Error(ErrorEvent),
  Unknown(UnknownEvent),
}

#[derive(Debug, Serialize)]
pub struct SessionStarted {
  pub provider: Provider,
  pub session_id: String,
  pub cwd: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderChanged {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub native_id: Option<String>,
  pub native_parent_id: Option<String>,
  pub model_provider: Option<String>,
  pub model_id: Option<String>,
  pub thinking_level: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MessageEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message_id: Option<String>,
  pub parent_id: Option<String>,
  pub role: Role,
  pub phase: Phase,
  pub text: String,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReasoningEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message_id: Option<String>,
  pub parent_id: Option<String>,
  pub phase: Phase,
  pub text: Option<String>,
  pub summary: Option<String>,
  pub encrypted_content: Option<String>,
  pub signature: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GoalUpdated {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub turn_id: Option<String>,
  pub goal: Option<Value>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolCallEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message_id: Option<String>,
  pub parent_id: Option<String>,
  pub tool_call_id: Option<String>,
  pub tool_name: Option<String>,
  pub tool_kind: ToolKind,
  pub summary: Option<ToolSummary>,
  pub phase: Phase,
  pub input: Option<Value>,
  pub output: Option<Value>,
  pub is_error: Option<bool>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message: String,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UnknownEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub native_type: Option<String>,
  pub timestamp: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
  Pi,
  Codex,
  #[serde(rename = "opencode")]
  OpenCode,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum Role {
  User,
  Assistant,
  System,
  Tool,
  Unknown,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum Phase {
  Started,
  Delta,
  Updated,
  Finished,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
  Shell,
  FileRead,
  FileWrite,
  FileEdit,
  Search,
  Web,
  Task,
  Unknown,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolSummary {
  Shell {
    command: Option<String>,
    cwd: Option<String>,
    exit_code: Option<i64>,
  },
  FileRead {
    path: Option<String>,
  },
  FileWrite {
    path: Option<String>,
    bytes: Option<u64>,
  },
  FileEdit {
    path: Option<String>,
    added: Option<u64>,
    removed: Option<u64>,
  },
  Search {
    query: Option<String>,
  },
  Web {
    url: Option<String>,
  },
  Task {
    title: Option<String>,
  },
}

pub fn tool_kind_for_name(name: &str) -> ToolKind {
  let normalized = name.rsplit('.').next().unwrap_or(name).to_ascii_lowercase();
  match normalized.as_str() {
    "bash" | "exec" | "exec_command" | "local_shell" | "shell" | "terminal" => ToolKind::Shell,
    "read" | "read_file" | "view" => ToolKind::FileRead,
    "write" | "write_file" | "create_file" => ToolKind::FileWrite,
    "edit" | "apply_patch" | "patch" | "str_replace" => ToolKind::FileEdit,
    "grep" | "rg" | "search" | "find" | "web_search" => ToolKind::Search,
    "open" | "fetch" => ToolKind::Web,
    "task" | "todo" | "update_plan" => ToolKind::Task,
    _ => ToolKind::Unknown,
  }
}

pub fn tool_kind_for_optional_name(name: Option<&str>) -> ToolKind {
  name.map(tool_kind_for_name).unwrap_or(ToolKind::Unknown)
}

pub fn tool_summary_for_input(name: &str, input: &Value) -> Option<ToolSummary> {
  tool_summary_for_io(Some(name), Some(input), None)
}

pub fn tool_summary_for_io(name: Option<&str>, input: Option<&Value>, output: Option<&Value>) -> Option<ToolSummary> {
  let kind = tool_kind_for_optional_name(name);
  match kind {
    ToolKind::Shell => Some(ToolSummary::Shell {
      command: input.and_then(shell_command_from_value),
      cwd: input.and_then(|input| string_field(input, "cwd").or_else(|| string_field(input, "workdir"))),
      exit_code: output.and_then(output_exit_code),
    }),
    ToolKind::FileRead => Some(ToolSummary::FileRead {
      path: input.and_then(path_field),
    }),
    ToolKind::FileWrite => Some(ToolSummary::FileWrite {
      path: input.and_then(path_field),
      bytes: None,
    }),
    ToolKind::FileEdit => Some(ToolSummary::FileEdit {
      path: input.and_then(patch_path),
      added: input.and_then(|input| patch_line_count(input, '+')),
      removed: input.and_then(|input| patch_line_count(input, '-')),
    }),
    ToolKind::Search => Some(ToolSummary::Search {
      query: input.and_then(|input| {
        string_field(input, "query")
          .or_else(|| string_field(input, "q"))
          .or_else(|| string_field(input, "pattern"))
      }),
    }),
    ToolKind::Web => Some(ToolSummary::Web {
      url: input.and_then(|input| string_field(input, "url").or_else(|| string_field(input, "ref_id"))),
    }),
    ToolKind::Task => Some(ToolSummary::Task {
      title: input.and_then(|input| string_field(input, "title").or_else(|| string_field(input, "step"))),
    }),
    ToolKind::Unknown => None,
  }
}

pub fn patch_summary(value: &Value) -> ToolSummary {
  ToolSummary::FileEdit {
    path: patch_path(value),
    added: patch_line_count(value, '+'),
    removed: patch_line_count(value, '-'),
  }
}

pub fn shell_command_from_value(value: &Value) -> Option<String> {
  string_field(value, "cmd")
    .or_else(|| string_field(value, "command"))
    .or_else(|| string_array_field(value, "command").map(|parts| parts.join(" ")))
}

fn output_exit_code(value: &Value) -> Option<i64> {
  value
    .get("metadata")
    .and_then(|metadata| metadata.get("exit"))
    .and_then(Value::as_i64)
}

fn patch_path(value: &Value) -> Option<String> {
  first_object_key(value)
    .or_else(|| path_field(value))
    .or_else(|| value.as_str().and_then(patch_text_path))
    .or_else(|| {
      value
        .as_array()
        .and_then(|changes| changes.first())
        .and_then(path_field)
    })
}

fn patch_line_count(value: &Value, marker: char) -> Option<u64> {
  value.as_str().or_else(|| first_unified_diff(value)).map(|patch| {
    patch
      .lines()
      .filter(|line| {
        line.starts_with(marker) && !line.starts_with("+++") && !line.starts_with("---") && !line.starts_with("***")
      })
      .count() as u64
  })
}

fn first_unified_diff(value: &Value) -> Option<&str> {
  value
    .as_object()
    .and_then(|changes| changes.values().next())
    .and_then(|change| change.get("unified_diff"))
    .and_then(Value::as_str)
}

fn first_object_key(value: &Value) -> Option<String> {
  value.as_object().and_then(|object| object.keys().next()).cloned()
}

fn patch_text_path(patch: &str) -> Option<String> {
  patch.lines().find_map(|line| {
    line
      .strip_prefix("*** Update File: ")
      .or_else(|| line.strip_prefix("*** Add File: "))
      .or_else(|| line.strip_prefix("*** Delete File: "))
      .map(str::to_string)
  })
}

fn path_field(value: &Value) -> Option<String> {
  string_field(value, "path")
    .or_else(|| string_field(value, "file_path"))
    .or_else(|| string_field(value, "filepath"))
    .or_else(|| string_field(value, "file"))
}

fn string_field(value: &Value, field: &str) -> Option<String> {
  value.get(field).and_then(Value::as_str).map(str::to_string)
}

fn string_array_field(value: &Value, field: &str) -> Option<Vec<String>> {
  value.get(field).and_then(Value::as_array).map(|items| {
    items
      .iter()
      .filter_map(Value::as_str)
      .map(str::to_string)
      .collect::<Vec<_>>()
  })
}
