use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_ATTEMPTS: usize = 4;

pub(crate) trait DiscordApi {
  async fn validate_destination(&mut self, guild_id: &str, channel_id: &str) -> Result<String, String>;
  async fn create_message(&mut self, channel_id: &str, message: DiscordMessage) -> Result<String, String>;
  async fn create_thread(&mut self, channel_id: &str, message_id: &str, name: &str) -> Result<String, String>;
}

pub struct DiscordClient {
  http: reqwest::Client,
  base_url: String,
}

impl DiscordClient {
  pub fn new(bot_token: &str) -> Result<Self, String> {
    let mut authorization = HeaderValue::from_str(&format!("Bot {bot_token}"))
      .map_err(|_| "Discord bot token contains invalid HTTP header characters".to_string())?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    let http = reqwest::Client::builder()
      .default_headers(headers)
      .user_agent(concat!("tokn-discord-pet/", env!("CARGO_PKG_VERSION")))
      .connect_timeout(Duration::from_secs(10))
      .timeout(Duration::from_secs(30))
      .build()
      .map_err(|err| format!("failed to build Discord HTTP client: {err}"))?;
    Ok(Self {
      http,
      base_url: DISCORD_API_BASE.to_string(),
    })
  }

  pub async fn validate_destination(&mut self, guild_id: &str, channel_id: &str) -> Result<String, String> {
    DiscordApi::validate_destination(self, guild_id, channel_id).await
  }

  async fn request_json(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value, String> {
    let url = format!("{}{path}", self.base_url);
    for attempt in 0..MAX_ATTEMPTS {
      let mut request = self.http.request(method.clone(), &url);
      if let Some(body) = body {
        request = request.json(body);
      }
      let response = match request.send().await {
        Ok(response) => response,
        Err(err) if err.is_builder() => return Err(format!("invalid Discord API request: {err}")),
        Err(_err) if attempt + 1 < MAX_ATTEMPTS => {
          tokio::time::sleep(retry_delay(attempt)).await;
          continue;
        }
        Err(err) => return Err(format!("Discord API request failed: {err}")),
      };
      let status = response.status();
      let response_body = response
        .text()
        .await
        .map_err(|err| format!("failed to read Discord API response: {err}"))?;
      if status.is_success() {
        return serde_json::from_str(&response_body).map_err(|err| format!("Discord API returned invalid JSON: {err}"));
      }
      if status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < MAX_ATTEMPTS {
        let retry_after = serde_json::from_str::<DiscordErrorBody>(&response_body)
          .ok()
          .and_then(|body| body.retry_after)
          .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
          .map(Duration::from_secs_f64)
          .unwrap_or_else(|| retry_delay(attempt))
          .min(Duration::from_secs(60));
        tokio::time::sleep(retry_after).await;
        continue;
      }
      if status.is_server_error() && attempt + 1 < MAX_ATTEMPTS {
        tokio::time::sleep(retry_delay(attempt)).await;
        continue;
      }
      return Err(discord_error(status, &response_body));
    }
    Err("Discord API request exhausted its retry budget".to_string())
  }
}

impl DiscordApi for DiscordClient {
  async fn validate_destination(&mut self, guild_id: &str, channel_id: &str) -> Result<String, String> {
    let user = self.request_json(Method::GET, "/users/@me", None).await?;
    let username = user
      .get("username")
      .and_then(Value::as_str)
      .unwrap_or("unknown bot")
      .to_string();
    let channel = self
      .request_json(Method::GET, &format!("/channels/{channel_id}"), None)
      .await?;
    let actual_guild = channel.get("guild_id").and_then(Value::as_str);
    if actual_guild != Some(guild_id) {
      return Err(format!(
        "Discord channel {channel_id} does not belong to configured guild {guild_id}"
      ));
    }
    Ok(username)
  }

  async fn create_message(&mut self, channel_id: &str, message: DiscordMessage) -> Result<String, String> {
    let body = serde_json::to_value(message).map_err(|err| format!("failed to serialize Discord message: {err}"))?;
    let response = self
      .request_json(Method::POST, &format!("/channels/{channel_id}/messages"), Some(&body))
      .await?;
    response
      .get("id")
      .and_then(Value::as_str)
      .map(str::to_string)
      .ok_or_else(|| "Discord create-message response did not contain an id".to_string())
  }

  async fn create_thread(&mut self, channel_id: &str, message_id: &str, name: &str) -> Result<String, String> {
    let body = json!({
      "name": name,
      "auto_archive_duration": 1440,
    });
    let response = self
      .request_json(
        Method::POST,
        &format!("/channels/{channel_id}/messages/{message_id}/threads"),
        Some(&body),
      )
      .await?;
    response
      .get("id")
      .and_then(Value::as_str)
      .map(str::to_string)
      .ok_or_else(|| "Discord create-thread response did not contain an id".to_string())
  }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct DiscordMessage {
  pub(crate) embeds: Vec<DiscordEmbed>,
  pub(crate) allowed_mentions: AllowedMentions,
}

impl DiscordMessage {
  pub(crate) fn new(title: String, description: String, color: u32) -> Self {
    Self {
      embeds: vec![DiscordEmbed {
        title,
        description,
        color,
      }],
      allowed_mentions: AllowedMentions { parse: Vec::new() },
    }
  }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct DiscordEmbed {
  pub(crate) title: String,
  pub(crate) description: String,
  pub(crate) color: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct AllowedMentions {
  pub(crate) parse: Vec<String>,
}

#[derive(Deserialize)]
struct DiscordErrorBody {
  message: Option<String>,
  code: Option<i64>,
  retry_after: Option<f64>,
}

fn retry_delay(attempt: usize) -> Duration {
  Duration::from_millis(250 * (1_u64 << attempt.min(4)))
}

fn discord_error(status: StatusCode, body: &str) -> String {
  let parsed = serde_json::from_str::<DiscordErrorBody>(body).ok();
  let message = parsed
    .as_ref()
    .and_then(|body| body.message.as_deref())
    .unwrap_or("unknown Discord API error");
  let code = parsed
    .as_ref()
    .and_then(|body| body.code)
    .map(|code| format!(" code {code}"))
    .unwrap_or_default();
  format!("Discord API returned {status}{code}: {message}")
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::DiscordMessage;

  #[test]
  fn outbound_messages_disable_all_mentions() {
    let message = DiscordMessage::new("User".to_string(), "@everyone hello".to_string(), 1);
    let value = serde_json::to_value(message).unwrap();

    assert_eq!(value["allowed_mentions"], json!({ "parse": [] }));
  }
}
