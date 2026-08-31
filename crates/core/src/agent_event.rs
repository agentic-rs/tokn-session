use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
  SessionStarted(SessionStarted),
  ProviderChanged(ProviderChanged),
  SessionSettingsApplied(SessionSettingsApplied),
  Message(MessageEvent),
  Reasoning(ReasoningEvent),
  GoalUpdated(GoalUpdated),
  AgentActivity(AgentActivity),
  ToolCall(ToolCallEvent),
  Lifecycle(LifecycleEvent),
  Usage(UsageEvent),
  Metadata(MetadataEvent),
  Error(ErrorEvent),
  Unknown(UnknownEvent),
}

impl AgentEvent {
  /// Human-facing consumers honor explicit visibility even when a hidden Pi
  /// extension message has an unsupported shape. Machine export stays lossless.
  pub fn is_hidden(&self) -> bool {
    match self {
      Self::Message(event) => event.provenance.as_ref().and_then(|source| source.display) == Some(false),
      Self::Reasoning(event) => event.provenance.as_ref().and_then(|source| source.display) == Some(false),
      Self::Unknown(event) if matches!(event.provider, Provider::Pi) => event
        .native
        .as_ref()
        .is_some_and(|native| native["type"] == "custom_message" && native["display"] == false),
      _ => false,
    }
  }
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
pub struct SessionSettingsApplied {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub model_provider: Option<String>,
  pub model_id: Option<String>,
  pub service_tier: Option<String>,
  pub cwd: Option<String>,
  pub reasoning_effort: Option<String>,
  pub reasoning_summary: Option<String>,
  pub personality: Option<String>,
  pub collaboration_mode: Option<String>,
  pub approval_policy: Option<String>,
  pub approvals_reviewer: Option<String>,
  pub active_permission_profile_id: Option<String>,
  pub native: Option<Value>,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MessageEvent {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub provenance: Option<MessageProvenance>,
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message_id: Option<String>,
  pub parent_id: Option<String>,
  pub role: Role,
  pub delivery: MessageDelivery,
  pub phase: Phase,
  pub text: String,
  pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReasoningEvent {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub provenance: Option<MessageProvenance>,
  pub provider: Provider,
  pub session_id: Option<String>,
  pub message_id: Option<String>,
  pub parent_id: Option<String>,
  pub phase: Phase,
  pub text: Option<String>,
  pub summary: Option<String>,
  /// The provider deliberately withheld the reasoning text. This is distinct
  /// from surface visibility: redacted reasoning remains part of the event
  /// stream and can be represented without exposing its content.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub redacted: Option<bool>,
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
pub struct AgentActivity {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub event_id: Option<String>,
  pub actor_session_id: Option<String>,
  pub actor_agent_path: Option<String>,
  pub target_session_id: Option<String>,
  pub target_agent_path: Option<String>,
  pub kind: String,
  pub occurred_at_ms: Option<u64>,
  pub native: Option<Value>,
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

/// Provider-native attribution and surface edits, not extra conversation text.
#[derive(Clone, Debug, Serialize)]
pub struct MessageProvenance {
  pub source: Value,
  /// Explicit provider visibility; absent means visible. JSONL retains content.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub display: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub native: Option<Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub surface_op: Option<Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source_event_seqs: Option<Vec<u64>>,
}

#[derive(Debug, Serialize)]
pub struct LifecycleEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub turn_id: String,
  pub step_id: Option<String>,
  pub scope: LifecycleScope,
  pub phase: Phase,
  /// Closing a step alone does not imply success; absent means unspecified.
  pub outcome: Option<LifecycleOutcome>,
  pub native: Value,
  pub timestamp: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleScope {
  Turn,
  Step,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleOutcome {
  Completed,
  Cancelled,
  Interrupted,
  Blocked,
  Failed,
  TokenLimit,
}

/// Accounting scope is explicit: session snapshots replace rather than add.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageKind {
  ModelCall,
  OperationTotal,
  SessionSnapshot,
}

#[derive(Debug, Serialize)]
pub struct UsageEvent {
  pub kind: UsageKind,
  pub provider: Provider,
  pub session_id: Option<String>,
  pub turn_id: Option<String>,
  pub step_id: Option<String>,
  pub message_id: Option<String>,
  /// Provider record identity when available, including non-message operations.
  pub record_id: Option<String>,
  /// Total input, including cache reads and writes. Cache fields are subsets.
  pub input_tokens: u64,
  pub output_tokens: u64,
  /// Total when known; native estimates need not equal the sum of the counters.
  pub total_tokens: Option<u64>,
  pub cache_read_tokens: Option<u64>,
  pub cache_write_tokens: Option<u64>,
  /// Provider-reported reasoning count; do not add it to output_tokens.
  pub reasoning_tokens: Option<u64>,
  /// Original usage object (not a duplicate of the entire assistant message).
  pub native: Value,
  pub timestamp: Option<String>,
}

/// Recognized non-conversation records. Unknown is reserved for unsupported or
/// malformed shapes; metadata must only be emitted after shape validation.
#[derive(Debug, Serialize)]
pub struct MetadataEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub kind: MetadataKind,
  pub native_type: String,
  pub summary: String,
  pub native: Value,
  pub timestamp: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataKind {
  Session,
  Configuration,
  Context,
  Queue,
  Diagnostic,
  Stream,
}

#[derive(Debug, Serialize)]
pub struct UnknownEvent {
  pub provider: Provider,
  pub session_id: Option<String>,
  pub native_type: Option<String>,
  pub native: Option<Value>,
  pub timestamp: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
  Dsh,
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
pub enum MessageDelivery {
  Commentary,
  Final,
  Unspecified,
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
    "ls" | "list_dir" | "list_directory" | "read" | "read_file" | "view" => ToolKind::FileRead,
    "write" | "write_file" | "create_file" => ToolKind::FileWrite,
    "edit" | "apply_patch" | "patch" | "str_replace" => ToolKind::FileEdit,
    "code_search" | "file_search" | "find" | "glob" | "grep" | "rg" | "search" | "tool_search" | "web_search" => {
      ToolKind::Search
    }
    "fetch" | "fetch_content" | "get_search_content" | "open" | "web_fetch" => ToolKind::Web,
    "followup_task" | "send_message" | "spawn_agent" | "subagent" | "task" | "todo" | "update_plan" | "wait"
    | "wait_agent" => ToolKind::Task,
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
      bytes: input.and_then(|input| {
        input
          .get("content")
          .and_then(Value::as_str)
          .map(|content| content.len() as u64)
      }),
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
          .or_else(|| joined_string_array_field(input, "queries"))
      }),
    }),
    ToolKind::Web => Some(ToolSummary::Web {
      url: input.and_then(|input| string_field(input, "url").or_else(|| string_field(input, "ref_id"))),
    }),
    ToolKind::Task => Some(ToolSummary::Task {
      title: input.and_then(|input| {
        string_field(input, "title")
          .or_else(|| string_field(input, "task_name"))
          .or_else(|| string_field(input, "prompt"))
          .or_else(|| string_field(input, "message"))
          .or_else(|| string_field(input, "step"))
      }),
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
    .or_else(|| value.get("exit_code").and_then(Value::as_i64))
    .or_else(|| value.get("exitCode").and_then(Value::as_i64))
    .or_else(|| value.get("details").and_then(output_exit_code))
}

fn patch_path(value: &Value) -> Option<String> {
  path_field(value)
    .or_else(|| first_change_map_path(value))
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
  value.as_object().and_then(|changes| {
    changes
      .values()
      .find_map(|change| change.get("unified_diff").and_then(Value::as_str))
  })
}

fn first_change_map_path(value: &Value) -> Option<String> {
  value.as_object().and_then(|changes| {
    changes.iter().find_map(|(path, change)| {
      let change = change.as_object()?;
      let has_unified_diff = change.get("unified_diff").is_some_and(Value::is_string);
      let has_known_type = matches!(
        change.get("type").and_then(Value::as_str),
        Some("add" | "delete" | "update")
      );
      (has_unified_diff || has_known_type).then(|| path.clone())
    })
  })
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

fn joined_string_array_field(value: &Value, field: &str) -> Option<String> {
  string_array_field(value, field).and_then(|items| (!items.is_empty()).then(|| items.join(", ")))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn redacted_reasoning_remains_visible() {
    let event = AgentEvent::Reasoning(ReasoningEvent {
      provenance: None,
      provider: Provider::Pi,
      session_id: Some("session-1".to_string()),
      message_id: Some("message-1".to_string()),
      parent_id: None,
      phase: Phase::Finished,
      text: None,
      summary: None,
      redacted: Some(true),
      encrypted_content: None,
      signature: None,
      timestamp: None,
    });

    assert!(!event.is_hidden());
    let serialized = serde_json::to_value(event).expect("reasoning event should serialize");
    assert_eq!(serialized["redacted"], true);
  }

  #[test]
  fn classifies_known_tool_families_without_treating_user_input_as_a_task() {
    for name in ["ls", "list_dir", "list_directory"] {
      assert!(matches!(tool_kind_for_name(name), ToolKind::FileRead));
    }
    for name in ["code_search", "file_search", "glob", "tool_search"] {
      assert!(matches!(tool_kind_for_name(name), ToolKind::Search));
    }
    for name in ["fetch_content", "get_search_content", "web_fetch"] {
      assert!(matches!(tool_kind_for_name(name), ToolKind::Web));
    }
    for name in [
      "subagent",
      "spawn_agent",
      "followup_task",
      "send_message",
      "wait",
      "wait_agent",
    ] {
      assert!(matches!(tool_kind_for_name(name), ToolKind::Task));
    }

    assert!(matches!(tool_kind_for_name("ask_user_question"), ToolKind::Unknown));
  }

  #[test]
  fn summarizes_multi_query_searches_and_file_write_bytes() {
    let search = serde_json::json!({ "queries": ["alpha", "beta"] });
    assert!(matches!(
      tool_summary_for_input("code_search", &search),
      Some(ToolSummary::Search { query: Some(query) }) if query == "alpha, beta"
    ));

    let write = serde_json::json!({ "path": "notes.txt", "content": "hello 🦀" });
    assert!(matches!(
      tool_summary_for_input("write", &write),
      Some(ToolSummary::FileWrite {
        path: Some(path),
        bytes: Some(10),
      }) if path == "notes.txt"
    ));
  }

  #[test]
  fn summarizes_collaboration_tasks_from_provider_specific_fields() {
    for (name, input, expected) in [
      (
        "spawn_agent",
        serde_json::json!({ "task_name": "reviewer" }),
        "reviewer",
      ),
      (
        "subagent",
        serde_json::json!({ "prompt": "Review the diff" }),
        "Review the diff",
      ),
      (
        "send_message",
        serde_json::json!({ "message": "Please retry" }),
        "Please retry",
      ),
      ("update_plan", serde_json::json!({ "step": "Run tests" }), "Run tests"),
    ] {
      assert!(matches!(
        tool_summary_for_input(name, &input),
        Some(ToolSummary::Task { title: Some(title) }) if title == expected
      ));
    }
  }

  #[test]
  fn shell_result_summary_reads_exit_metadata_through_a_provider_wrapper() {
    let output = serde_json::json!({
      "content": [{ "type": "text", "text": "failed" }],
      "details": { "metadata": { "exit": 2 } },
    });

    assert!(matches!(
      tool_summary_for_io(Some("bash"), None, Some(&output)),
      Some(ToolSummary::Shell {
        command: None,
        cwd: None,
        exit_code: Some(2),
      })
    ));
  }

  #[test]
  fn file_edit_summary_prefers_an_explicit_path_over_edit_fields() {
    let input = serde_json::json!({
      "oldText": "before",
      "newText": "after",
      "path": "src/lib.rs",
    });

    assert!(matches!(
      tool_summary_for_input("edit", &input),
      Some(ToolSummary::FileEdit {
        path: Some(path),
        added: None,
        removed: None,
      }) if path == "src/lib.rs"
    ));
  }

  #[test]
  fn file_edit_summary_keeps_change_map_path_and_counts() {
    let input = serde_json::json!({
      "src/main.rs": {
        "unified_diff": "@@ -1 +1 @@\n-before\n+after\n",
      },
    });

    assert!(matches!(
      tool_summary_for_input("apply_patch", &input),
      Some(ToolSummary::FileEdit {
        path: Some(path),
        added: Some(1),
        removed: Some(1),
      }) if path == "src/main.rs"
    ));

    let add = serde_json::json!({
      "src/new.rs": {
        "type": "add",
        "content": "fn main() {}\n",
      },
    });
    assert!(matches!(
      tool_summary_for_input("apply_patch", &add),
      Some(ToolSummary::FileEdit {
        path: Some(path),
        ..
      }) if path == "src/new.rs"
    ));
  }
}
