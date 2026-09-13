#[cfg(target_os = "macos")]
mod apple;

use serde::{Deserialize, Serialize};

const MAX_TEXTS: usize = 128;
const MAX_INPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize)]
pub struct TranslationStatus {
  pub available: bool,
  pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationRequest {
  pub request_id: String,
  pub texts: Vec<String>,
  pub target_language: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TranslationResponse {
  pub texts: Vec<String>,
}

impl TranslationRequest {
  fn validate(&self) -> Result<(), String> {
    validate_request_id(&self.request_id)?;
    if self.target_language != "zh-Hans" {
      return Err("The translation target must be Simplified Chinese (zh-Hans).".into());
    }
    if self.texts.is_empty() || self.texts.len() > MAX_TEXTS {
      return Err(format!("Translation requires between 1 and {MAX_TEXTS} text segments."));
    }
    if self.texts.iter().any(|text| text.trim().is_empty()) {
      return Err("Translation segments must contain text.".into());
    }
    if self.texts.iter().map(String::len).sum::<usize>() > MAX_INPUT_BYTES {
      return Err("Translation requests cannot exceed 64 KiB.".into());
    }
    Ok(())
  }
}

pub fn validate_request_id(request_id: &str) -> Result<(), String> {
  if request_id.is_empty()
    || request_id.len() > 128
    || !request_id
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
  {
    return Err("Invalid translation request ID.".into());
  }
  Ok(())
}

pub fn status() -> TranslationStatus {
  #[cfg(target_os = "macos")]
  let available = apple::available();
  #[cfg(not(target_os = "macos"))]
  let available = false;
  TranslationStatus {
    available,
    reason: (!available).then(|| "Apple Translation requires the Mac desktop app on macOS 15 or later.".into()),
  }
}

pub async fn translate(
  window: tauri::WebviewWindow,
  request: TranslationRequest,
) -> Result<TranslationResponse, String> {
  request.validate()?;
  if let Some(reason) = status().reason {
    return Err(reason);
  }
  #[cfg(target_os = "macos")]
  return apple::translate(window, request).await;
  #[cfg(not(target_os = "macos"))]
  {
    let _ = (window, request);
    Err("Apple Translation is unavailable on this platform.".into())
  }
}

pub fn cancel(window: tauri::WebviewWindow, request_id: &str) -> Result<(), String> {
  validate_request_id(request_id)?;
  #[cfg(target_os = "macos")]
  apple::cancel(window, request_id);
  #[cfg(not(target_os = "macos"))]
  let _ = window;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn request() -> TranslationRequest {
    TranslationRequest {
      request_id: "response-123".into(),
      texts: vec!["Hello world".into()],
      target_language: "zh-Hans".into(),
    }
  }

  #[test]
  fn validates_translation_request_limits_and_language() {
    let mut request = request();
    assert!(request.validate().is_ok());
    request.texts = vec!["hello".into(); MAX_TEXTS + 1];
    assert!(request.validate().is_err());
    request.texts = vec!["界".repeat(MAX_INPUT_BYTES / 3 + 1)];
    assert!(request.validate().is_err());
    request.texts = vec![" \n".into()];
    assert!(request.validate().is_err());
    request.texts = vec!["hello".into()];
    request.target_language = "en".into();
    assert!(request.validate().is_err());
  }

  #[test]
  fn validates_cancellation_identifiers() {
    for id in ["", " ", "a/b", "hello\0world"] {
      assert!(validate_request_id(id).is_err());
    }
    assert!(validate_request_id("message_123-456").is_ok());
  }
}
