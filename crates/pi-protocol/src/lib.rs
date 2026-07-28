//! Tolerant wire types for persisted Pi session JSONL.
//!
//! This crate models the session-file protocol consumed by `tokn-session`. It
//! keeps stable fields typed, retains volatile extension data as JSON, and
//! preserves unknown records so newer Pi versions cannot make a session
//! unreadable solely by adding a record, message role, or content-block type.

mod session;

pub use session::{
  ActiveToolsChangeItem, AssistantMessage, BranchSummaryItem, CompactionItem, ContentBlock, CustomItem,
  CustomMessageItem, ErrorItem, ExtraFields, ImageContent, LabelItem, LeafItem, Message, MessageItem, ModelChangeItem,
  PiSessionItem, PiSessionLine, SessionHeader, SessionInfoItem, TextContent, ThinkingContent, ThinkingLevelChangeItem,
  ToolCallContent, ToolResultMessage, UnknownItem, UserContent, UserMessage,
};
