use serde::Deserialize;
use serde_json::Value;

#[derive(Debug)]
pub struct OpenCodeSessionRow {
  pub id: String,
  pub parent_id: Option<String>,
  pub directory: Option<String>,
  pub model: Option<Value>,
  pub time_created: Option<i64>,
  pub time_updated: Option<i64>,
}

#[derive(Debug)]
pub struct OpenCodeMessageRow {
  pub id: String,
  pub time_created: Option<i64>,
  pub data: OpenCodeMessage,
  pub parts: Vec<OpenCodePartRow>,
}

#[derive(Debug)]
pub struct OpenCodePartRow {
  pub time_created: Option<i64>,
  pub data: OpenCodePart,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeMessage {
  pub role: OpenCodeRole,
  #[serde(rename = "parentID")]
  pub parent_id: Option<String>,
  #[serde(rename = "modelID")]
  pub model_id: Option<String>,
  #[serde(rename = "providerID")]
  pub provider_id: Option<String>,
  pub model: Option<OpenCodeModel>,
  pub error: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCodeRole {
  User,
  Assistant,
  System,
  Tool,
  #[serde(other)]
  Unknown,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeModel {
  #[serde(rename = "modelID")]
  pub model_id: Option<String>,
  #[serde(rename = "providerID")]
  pub provider_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum OpenCodePart {
  Text {
    text: String,
  },
  Reasoning {
    text: String,
  },
  Tool {
    #[serde(rename = "callID")]
    call_id: Option<String>,
    tool: Option<String>,
    state: OpenCodeToolState,
  },
  StepStart {},
  StepFinish {},
  #[serde(untagged)]
  Unknown(Value),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum OpenCodeToolState {
  Pending {
    input: Option<Value>,
    raw: Option<String>,
  },
  Running {
    input: Option<Value>,
    title: Option<String>,
    metadata: Option<Value>,
  },
  Completed {
    input: Option<Value>,
    output: Option<String>,
    title: Option<String>,
    metadata: Option<Value>,
  },
  Error {
    input: Option<Value>,
    error: Option<String>,
    metadata: Option<Value>,
  },
  #[serde(untagged)]
  Unknown(Value),
}
