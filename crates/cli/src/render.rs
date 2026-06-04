use serde_json::Value;
use tokn_session_core::{AgentEvent, LoadedSession, Phase, Role, SessionRef, ToolCallEvent, ToolKind, ToolSummary};

pub fn render_session_list(sessions: &[SessionRef]) -> String {
  let mut output = String::new();
  output.push_str("id                                    updated_at                 messages  cwd\n");
  for session in sessions {
    output.push_str(&format!(
      "{:<36}  {:<25} {:>8}  {}\n",
      session.id,
      session.timestamp.as_deref().unwrap_or("-"),
      session.message_count,
      session.cwd.as_deref().unwrap_or("-"),
    ));
  }
  output
}

pub fn render_agent_jsonl(events: &[AgentEvent]) -> Result<String, String> {
  let mut output = String::new();
  for event in events {
    let line = serde_json::to_string(event).map_err(|err| format!("failed to serialize event: {err}"))?;
    output.push_str(&line);
    output.push('\n');
  }
  Ok(output)
}

pub fn render_pretty(session: &LoadedSession) -> String {
  let mut output = String::new();
  output.push_str(&format!("Session {}\n", session.reference.id));
  output.push_str(&format!("cwd: {}\n", session.reference.cwd.as_deref().unwrap_or("-")));
  output.push_str(&format!(
    "updated_at: {}\n\n",
    session.reference.timestamp.as_deref().unwrap_or("-")
  ));

  for event in &session.events {
    match event {
      AgentEvent::SessionStarted(_) => {}
      AgentEvent::ProviderChanged(event) => {
        if let Some(model_id) = &event.model_id {
          let provider = event.model_provider.as_deref().unwrap_or("model");
          output.push_str(&format!("[model] {provider}/{model_id}\n\n"));
        }
        if let Some(level) = &event.thinking_level {
          output.push_str(&format!("[thinking] {level}\n\n"));
        }
      }
      AgentEvent::Message(event) => {
        output.push_str(role_label(event.role));
        output.push('\n');
        write_indented(&mut output, &event.text);
        output.push('\n');
      }
      AgentEvent::Reasoning(event) => {
        if let Some(summary) = &event.summary {
          output.push_str("reasoning summary\n");
          write_indented(&mut output, summary);
          output.push('\n');
        }
        if let Some(text) = &event.text {
          output.push_str("reasoning\n");
          write_indented(&mut output, text);
          output.push('\n');
        }
      }
      AgentEvent::GoalUpdated(event) => {
        output.push_str("goal updated");
        if let Some(status) = event.goal.as_ref().and_then(|goal| goal_string(goal, "status")) {
          output.push_str(&format!(" [{status}]"));
        }
        if let Some(tokens_used) = event.goal.as_ref().and_then(|goal| goal_number(goal, "tokensUsed")) {
          output.push_str(&format!(" tokens={tokens_used}"));
        }
        if let Some(time_used_seconds) = event
          .goal
          .as_ref()
          .and_then(|goal| goal_number(goal, "timeUsedSeconds"))
        {
          output.push_str(&format!(" time={time_used_seconds}s"));
        }
        output.push('\n');
        if let Some(objective) = event.goal.as_ref().and_then(|goal| goal_string(goal, "objective")) {
          write_indented(&mut output, objective);
        } else if let Some(goal) = &event.goal {
          write_indented(&mut output, &goal.to_string());
        }
        output.push('\n');
      }
      AgentEvent::ToolCall(event) => {
        render_tool(&mut output, event);
      }
      AgentEvent::Error(event) => {
        output.push_str("error\n");
        write_indented(&mut output, &event.message);
        output.push('\n');
      }
      AgentEvent::Unknown(event) => {
        output.push_str(&format!(
          "unknown {}\n\n",
          event.native_type.as_deref().unwrap_or("event")
        ));
      }
    }
  }

  output
}

fn render_tool(output: &mut String, event: &ToolCallEvent) {
  if let Some(line) = render_tool_summary(event) {
    output.push_str(&line);
    output.push('\n');
    if let Some(detail) = render_tool_detail(event) {
      write_indented(output, &detail);
    }
    output.push('\n');
    return;
  }

  output.push_str("tool");
  if let Some(name) = &event.tool_name {
    output.push(' ');
    output.push_str(name);
  }
  if let Some(id) = &event.tool_call_id {
    output.push_str(" #");
    output.push_str(id);
  }
  if event.is_error == Some(true) {
    output.push_str(" error");
  }
  output.push('\n');
  if let Some(input) = &event.input {
    write_indented(output, &format!("input: {input}"));
  }
  if let Some(output_value) = &event.output {
    write_indented(output, &format!("output: {output_value}"));
  }
  output.push('\n');
}

fn render_tool_summary(event: &ToolCallEvent) -> Option<String> {
  let status = tool_status(event);
  let mut line = match &event.summary {
    Some(ToolSummary::Shell {
      command,
      cwd: _,
      exit_code,
    }) => {
      let mut line = format!("shell{status}");
      if let Some(exit_code) = exit_code {
        line.push_str(&format!(" exit={exit_code}"));
      }
      if let Some(command) = command {
        line.push(' ');
        line.push_str(command);
      }
      line
    }
    Some(ToolSummary::FileRead { path }) => format!("read{status} {}", path.as_deref().unwrap_or("-")),
    Some(ToolSummary::FileWrite { path, bytes }) => {
      let mut line = format!("write{status} {}", path.as_deref().unwrap_or("-"));
      if let Some(bytes) = bytes {
        line.push_str(&format!(" {bytes}b"));
      }
      line
    }
    Some(ToolSummary::FileEdit { path, added, removed }) => {
      let mut line = format!("edit{status} {}", path.as_deref().unwrap_or("-"));
      if added.is_some() || removed.is_some() {
        line.push_str(&format!(" +{} -{}", added.unwrap_or(0), removed.unwrap_or(0)));
      }
      line
    }
    Some(ToolSummary::Search { query }) => format!("search{status} {}", query.as_deref().unwrap_or("-")),
    Some(ToolSummary::Web { url }) => format!("web{status} {}", url.as_deref().unwrap_or("-")),
    Some(ToolSummary::Task { title }) => format!("task{status} {}", title.as_deref().unwrap_or("-")),
    None => match event.tool_kind {
      ToolKind::Shell => Some(format!("shell{status}")),
      ToolKind::FileRead => Some(format!("read{status}")),
      ToolKind::FileWrite => Some(format!("write{status}")),
      ToolKind::FileEdit => Some(format!("edit{status}")),
      ToolKind::Search => Some(format!("search{status}")),
      ToolKind::Web => Some(format!("web{status}")),
      ToolKind::Task => Some(format!("task{status}")),
      ToolKind::Unknown => None,
    }?,
  };
  append_tool_id(&mut line, event);
  Some(line)
}

fn append_tool_id(line: &mut String, event: &ToolCallEvent) {
  if let Some(id) = &event.tool_call_id {
    line.push_str(" #");
    line.push_str(id);
  }
}

fn render_tool_detail(event: &ToolCallEvent) -> Option<String> {
  if event.is_error == Some(true) {
    return event.output.as_ref().map(|output| format!("output: {output}"));
  }
  match event.phase {
    Phase::Started | Phase::Updated => None,
    Phase::Delta | Phase::Finished => None,
  }
}

fn tool_status(event: &ToolCallEvent) -> &'static str {
  if event.is_error == Some(true) {
    return " error";
  }
  match event.phase {
    Phase::Started => " started",
    Phase::Updated => " running",
    Phase::Delta => " delta",
    Phase::Finished => "",
  }
}

fn goal_string<'a>(goal: &'a Value, field: &str) -> Option<&'a str> {
  goal.get(field).and_then(Value::as_str)
}

fn goal_number(goal: &Value, field: &str) -> Option<u64> {
  goal.get(field).and_then(Value::as_u64)
}

fn role_label(role: Role) -> &'static str {
  match role {
    Role::User => "user",
    Role::Assistant => "assistant",
    Role::System => "system",
    Role::Tool => "tool",
    Role::Unknown => "message",
  }
}

fn write_indented(output: &mut String, text: &str) {
  for line in text.trim_matches('\n').lines() {
    output.push_str("  ");
    output.push_str(line);
    output.push('\n');
  }
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use serde_json::json;
  use tokn_session_core::{AgentEvent, LoadedSession, Provider, SessionRef};

  use super::*;

  #[test]
  fn render_pretty_summarizes_shell_tools() {
    let session = loaded_session(vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id: Some("session".to_string()),
      message_id: None,
      parent_id: None,
      tool_call_id: Some("call".to_string()),
      tool_name: Some("exec_command".to_string()),
      tool_kind: ToolKind::Shell,
      summary: Some(ToolSummary::Shell {
        command: Some("cargo test".to_string()),
        cwd: None,
        exit_code: None,
      }),
      phase: Phase::Started,
      input: Some(json!(["cargo", "test"])),
      output: None,
      is_error: None,
      timestamp: None,
    })]);

    let output = render_pretty(&session);

    assert!(output.contains("shell started cargo test #call\n"));
    assert!(!output.contains("input:"));
  }

  #[test]
  fn render_pretty_summarizes_file_tools() {
    let session = loaded_session(vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id: Some("session".to_string()),
      message_id: None,
      parent_id: None,
      tool_call_id: Some("call".to_string()),
      tool_name: Some("apply_patch".to_string()),
      tool_kind: ToolKind::FileEdit,
      summary: Some(ToolSummary::FileEdit {
        path: Some("crates/core/src/agent_event.rs".to_string()),
        added: Some(4),
        removed: Some(1),
      }),
      phase: Phase::Finished,
      input: None,
      output: None,
      is_error: None,
      timestamp: None,
    })]);

    let output = render_pretty(&session);

    assert!(output.contains("edit crates/core/src/agent_event.rs +4 -1 #call\n"));
  }

  #[test]
  fn render_pretty_keeps_unknown_tool_payloads_visible() {
    let session = loaded_session(vec![AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Pi,
      session_id: Some("session".to_string()),
      message_id: None,
      parent_id: None,
      tool_call_id: Some("call".to_string()),
      tool_name: Some("mystery".to_string()),
      tool_kind: ToolKind::Unknown,
      summary: None,
      phase: Phase::Finished,
      input: Some(json!({ "value": 1 })),
      output: Some(json!({ "ok": true })),
      is_error: None,
      timestamp: None,
    })]);

    let output = render_pretty(&session);

    assert!(output.contains("tool mystery #call\n"));
    assert!(output.contains("input: {\"value\":1}\n"));
    assert!(output.contains("output: {\"ok\":true}\n"));
  }

  fn loaded_session(events: Vec<AgentEvent>) -> LoadedSession {
    LoadedSession {
      reference: SessionRef {
        id: "session".to_string(),
        path: PathBuf::from("session.jsonl"),
        cwd: None,
        timestamp: None,
        message_count: 0,
      },
      events,
    }
  }
}
