//! Tolerant wire types for persisted Codex rollout JSONL.
//!
//! This crate models the session-file protocol consumed by `tokn-session`. It
//! intentionally does not mirror Codex's internal Rust API. Stable fields are
//! typed, volatile subtrees remain JSON values, and unknown records retain
//! their original tags and payloads.

mod rollout;

pub use rollout::{
  AdditionalToolsItem, AgentMessageItem, CompactedItem, ContentItem, CustomToolCallItem, CustomToolCallOutputItem,
  EventMessage, FunctionCallItem, FunctionCallOutputItem, ImageGenerationCallItem, InterAgentCommunicationItem,
  InterAgentCommunicationMetadataItem, LocalShellCallItem, MessageItem, ReasoningItem, ResponseControlItem,
  ResponseItem, RolloutItem, RolloutLine, SessionGitInfo, SessionMetaItem, ToolSearchCallItem, ToolSearchOutputItem,
  TurnContextItem, UnknownItem, WebSearchCallItem, WorldStateItem,
};
