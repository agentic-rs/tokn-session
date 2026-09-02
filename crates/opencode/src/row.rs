use serde::Serialize;
use serde_json::Value;
use tokn_opencode_protocol::v1::{MessageData, PartData, SessionModel};

#[derive(Debug, Serialize)]
pub struct OpenCodeSessionRow {
  pub id: String,
  pub parent_id: Option<String>,
  pub directory: Option<String>,
  pub title: Option<String>,
  pub model: Option<SessionModel>,
  pub time_created: Option<i64>,
  pub time_updated: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct OpenCodeMessageRow {
  pub id: String,
  pub time_created: Option<i64>,
  pub data: MessageData,
  pub parts: Vec<OpenCodePartRow>,
}

#[derive(Debug, Serialize)]
pub struct OpenCodePartRow {
  pub id: String,
  pub time_created: Option<i64>,
  pub data: PartData,
}

#[derive(Debug, Serialize)]
pub struct OpenCodeSessionEntryRow {
  pub id: String,
  pub native_type: String,
  pub time_created: Option<i64>,
  pub data: Value,
}
