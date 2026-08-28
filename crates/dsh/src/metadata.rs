//! Display classification for explicitly supported plugin records. The wire
//! protocol remains open to arbitrary plugins; neither `ignorable` nor a name
//! prefix is sufficient evidence that an unfamiliar record is metadata.
use serde::Deserialize;
use serde_json::{Map, Value};
use tokn_dsh_protocol::{ContentBlock, EventRecord, UserMessage};
use tokn_session_core::MetadataKind;

pub(crate) fn classify(native: &Value) -> Option<(MetadataKind, String)> {
  // Validate the durable envelope as well as the plugin's required data.
  serde_json::from_value::<EventRecord<Value>>(native.clone()).ok()?;
  let record: KnownMetadata = serde_json::from_value(native.clone()).ok()?;
  Some(match record {
    KnownMetadata::Title { title, .. } => (MetadataKind::Session, format!("title: {title}")),
    KnownMetadata::Permission { preset } => (MetadataKind::Configuration, format!("permission preset: {preset}")),
    KnownMetadata::Sandbox { mode } => (MetadataKind::Configuration, format!("sandbox mode: {mode}")),
    KnownMetadata::Approval { policy } => (MetadataKind::Configuration, format!("approval policy: {policy}")),
    KnownMetadata::Inbox {
      target,
      start,
      removed_count,
      inserted,
      outcome,
    } => {
      if inserted.iter().any(|message| {
        message.role != "user"
          || message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Unknown(_)))
      }) {
        return None;
      }
      let target = match target {
        InboxTarget::NextTurn => "next-turn",
        InboxTarget::NextStep => "next-step",
      };
      (
        MetadataKind::Queue,
        format!(
          "inbox {target} at {start}: +{} -{}{}",
          inserted.len(),
          removed_count.unwrap_or(0),
          if outcome.is_some() { " (canceled)" } else { "" }
        ),
      )
    }
    KnownMetadata::TitleRequest { route, .. } => (
      MetadataKind::Diagnostic,
      format!("title model request {}/{}", route.provider, route.model),
    ),
    KnownMetadata::SearchRequest { body, .. } => {
      (MetadataKind::Diagnostic, format!("search model request {}", body.model))
    }
  })
}

// Unread fields below are intentionally deserialized for validation. Their
// complete values, including future extra fields, survive in MetadataEvent.native.
#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
enum KnownMetadata {
  #[serde(rename = "session/title")]
  Title {
    title: String,
    #[serde(rename = "messageSeqs")]
    message_seqs: Vec<u64>,
    source: TitleSource,
  },
  #[serde(rename = "permission/preset")]
  Permission { preset: String },
  #[serde(rename = "sandbox/mode")]
  Sandbox { mode: String },
  #[serde(rename = "approval/policy")]
  Approval { policy: String },
  #[serde(rename = "agent/inbox/spliced")]
  Inbox {
    target: InboxTarget,
    start: u64,
    #[serde(rename = "removedCount")]
    removed_count: Option<u64>,
    inserted: Vec<UserMessage>,
    outcome: Option<InboxOutcome>,
  },
  #[serde(rename = "session/title-llm-request")]
  TitleRequest {
    #[serde(rename = "titleProvider")]
    title_provider: String,
    #[serde(rename = "messageSeqs")]
    message_seqs: Vec<u64>,
    route: ModelRoute,
    system: String,
    messages: Vec<RequestMessage>,
    #[serde(rename = "maxTokens")]
    max_tokens: u64,
  },
  #[serde(rename = "web/deepseek-search-llm-request")]
  SearchRequest {
    endpoint: String,
    #[serde(rename = "apiVersion")]
    api_version: String,
    body: SearchBody,
  },
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum InboxTarget {
  NextTurn,
  NextStep,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum InboxOutcome {
  Canceled,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum TitleSource {
  Fallback,
  User,
  Provider {
    provider: String,
    model: Option<ModelRoute>,
  },
}

#[derive(Deserialize)]
struct ModelRoute {
  provider: String,
  model: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct RequestMessage {
  role: String,
  // Auxiliary request bodies are diagnostic payloads, not conversation events.
  content: Vec<Map<String, Value>>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct SearchBody {
  model: String,
  max_tokens: u64,
  messages: Vec<RequestMessage>,
  tools: Vec<SearchTool>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct SearchTool {
  #[serde(rename = "type")]
  tool_type: String,
  name: String,
  max_uses: u64,
}
