//! Conservative presentation policy for the viewer's bookkeeping filter.
//! Keep this separate from ingestion and pagination: filtering must not alter
//! source event keys, native inspection, or the normalized event stream.

use serde_json::Value;
use tokn_session_core::{AgentEvent, LifecycleOutcome, MetadataEvent, MetadataKind, Provider};

pub(super) fn is_bookkeeping(event: &AgentEvent) -> bool {
  match event {
    AgentEvent::SessionStarted(_) | AgentEvent::ProviderChanged(_) | AgentEvent::SessionSettingsApplied(_) => true,
    AgentEvent::Lifecycle(event) => {
      matches!(event.outcome, None | Some(LifecycleOutcome::Completed)) && !lifecycle_has_content(&event.native)
    }
    AgentEvent::Metadata(event) => routine_metadata(event),
    // In particular, encrypted deliveries and empty-looking usage/tool rows
    // still carry information. New event variants remain visible by default.
    _ => false,
  }
}

fn routine_metadata(event: &MetadataEvent) -> bool {
  // MetadataKind describes storage purpose, not whether content is present.
  // Context includes branch summaries, plans, hooks, and attachments; queue
  // records can contain undelivered user input; diagnostics can hold model
  // request bodies. Only known bookkeeping contracts belong in this list.
  match (event.provider, event.kind, event.native_type.as_str()) {
    (Provider::Codex, MetadataKind::Configuration, "turn_context")
    | (Provider::Codex, MetadataKind::Context, "world_state" | "inter_agent_communication_metadata")
    | (Provider::Codex, MetadataKind::Diagnostic, "event_msg.token_count")
    | (Provider::Pi, MetadataKind::Configuration, "active_tools_change")
    | (Provider::Pi, MetadataKind::Context, "leaf")
    | (Provider::Pi, MetadataKind::Session, "session_info" | "label")
    | (Provider::Dsh, MetadataKind::Configuration, "permission/preset" | "sandbox/mode" | "approval/policy")
    | (Provider::Dsh, MetadataKind::Context, "request/context")
    | (Provider::Dsh, MetadataKind::Session, "session/end-seed" | "session/title")
    | (Provider::WorkBuddy, MetadataKind::Session, "ai-title")
    | (Provider::OpenCode | Provider::ZCode, MetadataKind::Configuration, "runtime/bash_shell_selection") => true,
    (
      Provider::Codex,
      MetadataKind::Context,
      "event_msg.item_started.AgentMessage"
      | "event_msg.item_completed.AgentMessage"
      | "event_msg.item_started.SubAgentActivity",
    ) => !lifecycle_has_content(&event.native),
    _ => false,
  }
}

fn lifecycle_has_content(value: &Value) -> bool {
  match value {
    Value::Object(object) => object.iter().any(|(key, value)| match key.as_str() {
      // Some provider completion records still report Completed while retaining
      // an error in their native payload. Preserve those even if is_error is false.
      "error" if !value.is_null() => true,
      "text" | "last_agent_message" if meaningful_text(value) => true,
      _ => lifecycle_has_content(value),
    }),
    Value::Array(values) => values.iter().any(lifecycle_has_content),
    _ => false,
  }
}

fn meaningful_text(value: &Value) -> bool {
  match value {
    Value::Null => false,
    Value::String(text) => !text.trim().is_empty(),
    Value::Array(values) => !values.is_empty(),
    Value::Object(values) => !values.is_empty(),
    // Unexpected shapes under a text field should remain inspectable.
    Value::Bool(_) | Value::Number(_) => true,
  }
}
