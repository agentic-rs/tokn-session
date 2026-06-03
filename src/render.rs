use crate::agent_event::{AgentEvent, Role};
use crate::pi::session_source::{LoadedSession, SessionRef};

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
        output.push_str("reasoning\n");
        write_indented(&mut output, &event.text);
        output.push('\n');
      }
      AgentEvent::ToolCall(event) => {
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
          write_indented(&mut output, &format!("input: {input}"));
        }
        if let Some(output_value) = &event.output {
          write_indented(&mut output, &format!("output: {output_value}"));
        }
        output.push('\n');
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
