use std::path::Path;

use serde_json::Value;
use tokn_session_core::{
  AgentActivity, AgentEvent, LiveSessionEvent, LoadedSession, Phase, Role, SessionRef, SessionSettingsApplied,
  ToolCallEvent, ToolKind, ToolSummary,
};

pub struct EventDisplay {
  pub kind: &'static str,
  pub summary: String,
  pub detail: String,
}

pub fn display_event(event: &AgentEvent) -> EventDisplay {
  EventDisplay {
    kind: event_type(event),
    summary: render_event_summary(event),
    detail: render_event_pretty(event),
  }
}

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

pub fn render_live_event_pretty(event: &LiveSessionEvent) -> String {
  let mut output = String::new();
  match event {
    LiveSessionEvent::Started(event) => {
      output.push_str(&format!("Session {}\n", event.session_id));
      output.push_str(&format!("cwd: {}\n\n", event.cwd.as_deref().unwrap_or("-")));
    }
    LiveSessionEvent::Event(event) => output.push_str(&render_event_pretty(event)),
    LiveSessionEvent::Finished(event) => {
      output.push_str("session finished");
      if !event.success {
        output.push_str(" error");
      }
      if let Some(exit_code) = event.exit_code {
        output.push_str(&format!(" exit={exit_code}"));
      }
      output.push_str("\n\n");
    }
    LiveSessionEvent::Unknown(event) => {
      output.push_str(&format!(
        "unknown {}\n",
        event.native_type.as_deref().unwrap_or("event")
      ));
      if let Some(native) = &event.native {
        write_indented(&mut output, &format!("native: {native}"));
      }
      output.push('\n');
    }
  }
  output
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
    output.push_str(&render_event_pretty(event));
  }

  output
}

pub fn render_event_pretty(event: &AgentEvent) -> String {
  let mut output = String::new();
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
    AgentEvent::SessionSettingsApplied(event) => render_session_settings(&mut output, event),
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
    AgentEvent::AgentActivity(event) => {
      output.push_str(&render_agent_activity_summary(event));
      output.push_str("\n\n");
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
        "unknown {}\n",
        event.native_type.as_deref().unwrap_or("event")
      ));
      if let Some(native) = &event.native {
        write_indented(&mut output, &format!("native: {native}"));
      }
      output.push('\n');
    }
  }
  output
}

pub fn render_event_summary(event: &AgentEvent) -> String {
  match event {
    AgentEvent::SessionStarted(event) => format!("session started {}", event.session_id),
    AgentEvent::ProviderChanged(event) => {
      if let Some(model_id) = &event.model_id {
        let provider = event.model_provider.as_deref().unwrap_or("model");
        format!("model {provider}/{model_id}")
      } else if let Some(level) = &event.thinking_level {
        format!("thinking {level}")
      } else {
        "provider changed".to_string()
      }
    }
    AgentEvent::SessionSettingsApplied(event) => render_session_settings_summary(event),
    AgentEvent::Message(event) => format!("{} {}", role_label(event.role), first_line(&event.text)),
    AgentEvent::Reasoning(event) => {
      if let Some(summary) = &event.summary {
        format!("reasoning summary {}", first_line(summary))
      } else if let Some(text) = &event.text {
        format!("reasoning {}", first_line(text))
      } else {
        "reasoning encrypted".to_string()
      }
    }
    AgentEvent::GoalUpdated(event) => {
      let mut summary = "goal updated".to_string();
      if let Some(status) = event.goal.as_ref().and_then(|goal| goal_string(goal, "status")) {
        summary.push_str(&format!(" [{status}]"));
      }
      if let Some(objective) = event.goal.as_ref().and_then(|goal| goal_string(goal, "objective")) {
        summary.push(' ');
        summary.push_str(first_line(objective));
      }
      summary
    }
    AgentEvent::AgentActivity(event) => render_agent_activity_summary(event),
    AgentEvent::ToolCall(event) => render_tool_summary(event).unwrap_or_else(|| {
      let mut summary = "tool".to_string();
      if let Some(name) = &event.tool_name {
        summary.push(' ');
        summary.push_str(name);
      }
      append_tool_id(&mut summary, event);
      summary
    }),
    AgentEvent::Error(event) => format!("error {}", first_line(&event.message)),
    AgentEvent::Unknown(event) => format!("unknown {}", event.native_type.as_deref().unwrap_or("event")),
  }
}

pub fn event_type(event: &AgentEvent) -> &'static str {
  match event {
    AgentEvent::SessionStarted(_) => "session",
    AgentEvent::ProviderChanged(_) => "provider",
    AgentEvent::SessionSettingsApplied(_) => "settings",
    AgentEvent::Message(_) => "message",
    AgentEvent::Reasoning(_) => "reasoning",
    AgentEvent::GoalUpdated(_) => "goal",
    AgentEvent::AgentActivity(_) => "agent",
    AgentEvent::ToolCall(_) => "tool",
    AgentEvent::Error(_) => "error",
    AgentEvent::Unknown(_) => "unknown",
  }
}

fn render_agent_activity_summary(event: &AgentActivity) -> String {
  let target = event.target_agent_path.as_deref().unwrap_or("unknown agent");
  let mut summary = match event.actor_agent_path.as_deref() {
    Some(actor) => format!("{actor} → {target} {}", event.kind),
    None => match event.kind.as_str() {
      "started" => format!("agent started {target}"),
      "interacted" => format!("interaction with {target}"),
      "interrupted" => format!("agent interrupted {target}"),
      kind => format!("agent activity {kind} {target}"),
    },
  };
  if let Some(event_id) = &event.event_id {
    summary.push_str(" #");
    summary.push_str(event_id);
  }
  summary
}

fn render_session_settings(output: &mut String, event: &SessionSettingsApplied) {
  output.push_str("session settings applied\n");
  if let Some(model) = joined_values(event.model_provider.as_deref(), event.model_id.as_deref(), "/") {
    write_setting(output, "model", &model);
  }
  if let Some(service_tier) = &event.service_tier {
    write_setting(output, "service tier", service_tier);
  }
  if let Some(reasoning) = joined_values(
    event.reasoning_effort.as_deref(),
    event.reasoning_summary.as_deref(),
    " / ",
  ) {
    write_setting(output, "reasoning", &reasoning);
  }
  if let Some(mode) = &event.collaboration_mode {
    write_setting(output, "mode", mode);
  }
  if let Some(personality) = &event.personality {
    write_setting(output, "personality", personality);
  }
  if let Some(approval) = joined_values(
    event.approval_policy.as_deref(),
    event.approvals_reviewer.as_deref(),
    " / ",
  ) {
    write_setting(output, "approval", &approval);
  }
  if let Some(profile) = &event.active_permission_profile_id {
    write_setting(output, "permissions", profile);
  }
  if let Some(cwd) = &event.cwd {
    write_setting(output, "cwd", cwd);
  }
  output.push('\n');
}

fn render_session_settings_summary(event: &SessionSettingsApplied) -> String {
  let mut parts = vec!["settings".to_string()];
  match (&event.model_provider, &event.model_id) {
    (Some(provider), Some(model)) => parts.push(format!("model={provider}/{model}")),
    (Some(provider), None) => parts.push(format!("provider={provider}")),
    (None, Some(model)) => parts.push(format!("model={model}")),
    (None, None) => {}
  }
  if let Some(service_tier) = &event.service_tier {
    parts.push(format!("tier={service_tier}"));
  }
  if let Some(effort) = &event.reasoning_effort {
    parts.push(format!("effort={effort}"));
  }
  if let Some(mode) = &event.collaboration_mode {
    parts.push(format!("mode={mode}"));
  }
  if let Some(cwd) = event.cwd.as_deref().and_then(cwd_name) {
    parts.push(format!("cwd={cwd}"));
  }
  parts.join(" ")
}

fn write_setting(output: &mut String, label: &str, value: &str) {
  output.push_str("  ");
  output.push_str(label);
  output.push(' ');
  output.push_str(value);
  output.push('\n');
}

fn joined_values(first: Option<&str>, second: Option<&str>, separator: &str) -> Option<String> {
  match (first, second) {
    (Some(first), Some(second)) => Some(format!("{first}{separator}{second}")),
    (Some(first), None) => Some(first.to_string()),
    (None, Some(second)) => Some(second.to_string()),
    (None, None) => None,
  }
}

fn cwd_name(cwd: &str) -> Option<&str> {
  Path::new(cwd)
    .file_name()
    .and_then(|name| name.to_str())
    .filter(|name| !name.is_empty())
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

fn first_line(text: &str) -> &str {
  text.trim().lines().next().unwrap_or("")
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use serde_json::json;
  use tokn_session_core::{
    AgentActivity, AgentEvent, GoalUpdated, LoadedSession, MessageEvent, Provider, ProviderChanged, ReasoningEvent,
    SessionRef, SessionSettingsApplied, UnknownEvent,
  };

  use super::*;

  #[test]
  fn display_event_exposes_summary_and_detail() {
    let event = AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id: Some("session".to_string()),
      message_id: None,
      parent_id: None,
      tool_call_id: Some("call".to_string()),
      tool_name: Some("exec_command".to_string()),
      tool_kind: ToolKind::Shell,
      summary: Some(ToolSummary::Shell {
        command: Some("cargo check".to_string()),
        cwd: None,
        exit_code: None,
      }),
      phase: Phase::Finished,
      input: None,
      output: None,
      is_error: None,
      timestamp: None,
    });

    let display = display_event(&event);

    assert_eq!(display.kind, "tool");
    assert_eq!(display.summary, "shell cargo check #call");
    assert_eq!(display.detail, "shell cargo check #call\n\n");
  }

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
  fn render_event_summary_handles_message_first_line() {
    let event = AgentEvent::Message(MessageEvent {
      provider: Provider::Codex,
      session_id: Some("session".to_string()),
      message_id: None,
      parent_id: None,
      role: Role::Assistant,
      phase: Phase::Finished,
      text: "first line\nsecond line".to_string(),
      timestamp: None,
    });

    assert_eq!(render_event_summary(&event), "assistant first line");
  }

  #[test]
  fn renders_agent_activity_as_target_when_actor_is_unknown() {
    let event = agent_activity(None);

    assert_eq!(render_event_summary(&event), "interaction with /root #call-agent");
    assert_eq!(render_event_pretty(&event), "interaction with /root #call-agent\n\n");
  }

  #[test]
  fn renders_agent_activity_direction_when_actor_is_known() {
    let event = agent_activity(Some("/root/researcher"));

    assert_eq!(
      render_event_summary(&event),
      "/root/researcher → /root interacted #call-agent"
    );
  }

  #[test]
  fn render_pretty_handles_reasoning_and_goal_updates() {
    let session = loaded_session(vec![
      AgentEvent::Reasoning(ReasoningEvent {
        provider: Provider::Codex,
        session_id: Some("session".to_string()),
        message_id: None,
        parent_id: None,
        phase: Phase::Finished,
        text: Some("thinking".to_string()),
        summary: Some("summary".to_string()),
        encrypted_content: Some("ciphertext".to_string()),
        signature: None,
        timestamp: None,
      }),
      AgentEvent::GoalUpdated(GoalUpdated {
        provider: Provider::Codex,
        session_id: Some("session".to_string()),
        turn_id: Some("turn".to_string()),
        goal: Some(json!({
          "status": "complete",
          "objective": "finish tests",
          "tokensUsed": 12,
          "timeUsedSeconds": 3
        })),
        timestamp: None,
      }),
    ]);

    let output = render_pretty(&session);

    assert!(output.contains("reasoning summary\n  summary\n"));
    assert!(output.contains("reasoning\n  thinking\n"));
    assert!(!output.contains("ciphertext"));
    assert!(output.contains("goal updated [complete] tokens=12 time=3s\n  finish tests\n"));
  }

  #[test]
  fn render_pretty_handles_provider_changes() {
    let session = loaded_session(vec![AgentEvent::ProviderChanged(ProviderChanged {
      provider: Provider::Codex,
      session_id: Some("session".to_string()),
      native_id: None,
      native_parent_id: None,
      model_provider: Some("openai".to_string()),
      model_id: Some("gpt-5".to_string()),
      thinking_level: Some("high".to_string()),
      timestamp: None,
    })]);

    let output = render_pretty(&session);

    assert!(output.contains("[model] openai/gpt-5\n\n"));
    assert!(output.contains("[thinking] high\n\n"));
  }

  #[test]
  fn renders_session_settings_without_native_details() {
    let event = AgentEvent::SessionSettingsApplied(SessionSettingsApplied {
      provider: Provider::Codex,
      session_id: Some("session".to_string()),
      model_provider: Some("openai".to_string()),
      model_id: Some("gpt-5".to_string()),
      service_tier: Some("priority".to_string()),
      cwd: Some("/tmp/project".to_string()),
      reasoning_effort: Some("high".to_string()),
      reasoning_summary: Some("detailed".to_string()),
      personality: Some("friendly".to_string()),
      collaboration_mode: Some("default".to_string()),
      approval_policy: Some("on-request".to_string()),
      approvals_reviewer: Some("auto_review".to_string()),
      active_permission_profile_id: Some(":workspace".to_string()),
      native: Some(json!({
        "collaboration_mode": {
          "settings": {
            "developer_instructions": "sensitive instructions"
          }
        }
      })),
      timestamp: None,
    });

    assert_eq!(
      render_event_summary(&event),
      "settings model=openai/gpt-5 tier=priority effort=high mode=default cwd=project"
    );
    let pretty = render_event_pretty(&event);
    assert!(pretty.contains("session settings applied\n"));
    assert!(pretty.contains("  model openai/gpt-5\n"));
    assert!(pretty.contains("  reasoning high / detailed\n"));
    assert!(pretty.contains("  approval on-request / auto_review\n"));
    assert!(pretty.contains("  permissions :workspace\n"));
    assert!(!pretty.contains("sensitive instructions"));
  }

  #[test]
  fn render_event_summary_keeps_tool_ids() {
    let event = AgentEvent::ToolCall(ToolCallEvent {
      provider: Provider::Codex,
      session_id: Some("session".to_string()),
      message_id: None,
      parent_id: None,
      tool_call_id: Some("call".to_string()),
      tool_name: Some("exec_command".to_string()),
      tool_kind: ToolKind::Shell,
      summary: Some(ToolSummary::Shell {
        command: Some("cargo check".to_string()),
        cwd: None,
        exit_code: None,
      }),
      phase: Phase::Finished,
      input: None,
      output: None,
      is_error: None,
      timestamp: None,
    });

    assert_eq!(render_event_summary(&event), "shell cargo check #call");
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

  #[test]
  fn render_pretty_keeps_unknown_event_payloads_visible() {
    let session = loaded_session(vec![AgentEvent::Unknown(UnknownEvent {
      provider: Provider::Codex,
      session_id: Some("session".to_string()),
      native_type: Some("event_msg.new_native_event".to_string()),
      native: Some(json!({ "type": "new_native_event", "value": 123 })),
      timestamp: None,
    })]);

    let output = render_pretty(&session);

    assert!(output.contains("unknown event_msg.new_native_event\n"));
    assert!(output.contains("native: {\"type\":\"new_native_event\",\"value\":123}\n"));
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

  fn agent_activity(actor_agent_path: Option<&str>) -> AgentEvent {
    AgentEvent::AgentActivity(AgentActivity {
      provider: Provider::Codex,
      session_id: Some("child-session".to_string()),
      event_id: Some("call-agent".to_string()),
      actor_session_id: actor_agent_path.map(|_| "child-session".to_string()),
      actor_agent_path: actor_agent_path.map(str::to_string),
      target_session_id: Some("root-session".to_string()),
      target_agent_path: Some("/root".to_string()),
      kind: "interacted".to_string(),
      occurred_at_ms: Some(1_784_915_647_361),
      native: None,
      timestamp: Some("2026-07-24T17:54:07.361Z".to_string()),
    })
  }
}
