use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokn_session_core::{AgentEvent, MessageDelivery, Phase, Provider, Role};
use tokn_session_relay::{ProjectContext, RelayEvent};

use crate::discord::{DiscordApi, DiscordClient, DiscordMessage};

const EMBED_DESCRIPTION_LIMIT: usize = 3_900;
const USER_COLOR: u32 = 0x0058_65f2;
const FINAL_COLOR: u32 = 0x0057_f287;

pub struct DiscordPet {
  inner: Pet<DiscordClient>,
}

impl DiscordPet {
  pub fn new(api: DiscordClient, channel_id: String, state_path: PathBuf) -> Result<Self, String> {
    Ok(Self {
      inner: Pet::new(api, channel_id, state_path)?,
    })
  }

  pub async fn process(&mut self, relay: &RelayEvent) -> Result<(), String> {
    self.inner.process(relay).await
  }
}

struct Pet<A> {
  api: A,
  channel_id: String,
  state: PetState,
  state_path: PathBuf,
}

impl<A: DiscordApi> Pet<A> {
  fn new(api: A, channel_id: String, state_path: PathBuf) -> Result<Self, String> {
    Ok(Self {
      api,
      channel_id,
      state: PetState::load(&state_path)?,
      state_path,
    })
  }

  async fn process(&mut self, relay: &RelayEvent) -> Result<(), String> {
    if relay.session.parent_session_id.is_some() {
      return Ok(());
    }
    let AgentEvent::Message(message) = &relay.event else {
      return Ok(());
    };
    if !matches!(message.phase, Phase::Finished) || message.text.trim().is_empty() {
      return Ok(());
    }
    let kind = match (message.role, message.delivery) {
      (Role::User, _) => PublishedKind::User,
      (Role::Assistant, MessageDelivery::Final) => PublishedKind::Final,
      _ => return Ok(()),
    };

    let chunks = split_message(&message.text, EMBED_DESCRIPTION_LIMIT);
    let existing_thread = self.state.threads.get(&relay.topic).cloned();
    let had_existing_thread = existing_thread.is_some();
    let (thread_id, remaining) = if let Some(thread_id) = existing_thread {
      (thread_id, chunks.as_slice())
    } else {
      let first = chunks.first().expect("non-empty messages produce a chunk");
      let starter_id = self
        .api
        .create_message(&self.channel_id, kind.message(first, false))
        .await?;
      let thread_id = self
        .api
        .create_thread(&self.channel_id, &starter_id, &thread_name(relay))
        .await?;
      self.state.threads.insert(relay.topic.clone(), thread_id.clone());
      self.state.save(&self.state_path)?;
      (thread_id, &chunks[1..])
    };

    for (index, chunk) in remaining.iter().enumerate() {
      self
        .api
        .create_message(&thread_id, kind.message(chunk, index > 0 || !had_existing_thread))
        .await?;
    }
    Ok(())
  }
}

#[derive(Clone, Copy)]
enum PublishedKind {
  User,
  Final,
}

impl PublishedKind {
  fn message(self, text: &str, continued: bool) -> DiscordMessage {
    let (title, color) = match self {
      Self::User => ("User", USER_COLOR),
      Self::Final => ("Final", FINAL_COLOR),
    };
    let title = if continued {
      format!("{title} · continued")
    } else {
      title.to_string()
    };
    DiscordMessage::new(title, text.to_string(), color)
  }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PetState {
  threads: HashMap<String, String>,
}

impl PetState {
  fn load(path: &Path) -> Result<Self, String> {
    match fs::read(path) {
      Ok(contents) => serde_json::from_slice(&contents)
        .map_err(|err| format!("failed to parse Discord pet state {}: {err}", path.display())),
      Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
      Err(err) => Err(format!("failed to read Discord pet state {}: {err}", path.display())),
    }
  }

  fn save(&self, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).map_err(|err| {
        format!(
          "failed to create Discord pet state directory {}: {err}",
          parent.display()
        )
      })?;
    }
    let temporary = path.with_extension("json.tmp");
    let contents =
      serde_json::to_vec_pretty(self).map_err(|err| format!("failed to serialize Discord pet state: {err}"))?;
    fs::write(&temporary, contents)
      .map_err(|err| format!("failed to write Discord pet state {}: {err}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|err| format!("failed to replace Discord pet state {}: {err}", path.display()))
  }
}

fn split_message(text: &str, max_utf16_units: usize) -> Vec<String> {
  let mut chunks = Vec::new();
  let mut chunk = String::new();
  let mut units = 0;
  for character in text.chars() {
    let character_units = character.len_utf16();
    if units + character_units > max_utf16_units && !chunk.is_empty() {
      chunks.push(chunk);
      chunk = String::new();
      units = 0;
    }
    chunk.push(character);
    units += character_units;
  }
  if !chunk.is_empty() {
    chunks.push(chunk);
  }
  chunks
}

fn thread_name(relay: &RelayEvent) -> String {
  let project = relay
    .session
    .project
    .as_ref()
    .and_then(project_label)
    .or(relay.session.title.as_deref())
    .unwrap_or("session");
  let provider = match relay.session.provider {
    Provider::Codex => "codex",
    Provider::Pi => "pi",
    Provider::OpenCode => "opencode",
  };
  let session_id = abbreviate_id(&relay.session.session_id);
  truncate_utf16(&format!("{} · {provider}/{session_id}", single_line(project)), 100)
}

fn project_label(project: &ProjectContext) -> Option<&str> {
  project
    .project_name
    .as_deref()
    .or(project.folder_name.as_deref())
    .or(project.repository_name.as_deref())
    .or(project.name.as_deref())
}

fn single_line(value: &str) -> String {
  value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn abbreviate_id(value: &str) -> &str {
  value.get(..8).unwrap_or(value)
}

fn truncate_utf16(value: &str, max_units: usize) -> String {
  let mut output = String::new();
  let mut units = 0;
  for character in value.chars() {
    if units + character.len_utf16() > max_units {
      break;
    }
    output.push(character);
    units += character.len_utf16();
  }
  output
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use tempfile::TempDir;
  use tokn_session_core::MessageEvent;
  use tokn_session_relay::{ProjectContext, SessionContext};

  use super::*;

  #[derive(Default)]
  struct FakeDiscord {
    messages: Vec<(String, DiscordMessage)>,
    threads: Vec<(String, String, String)>,
  }

  impl DiscordApi for FakeDiscord {
    async fn validate_destination(&mut self, _guild_id: &str, _channel_id: &str) -> Result<String, String> {
      Ok("pet".to_string())
    }

    async fn create_message(&mut self, channel_id: &str, message: DiscordMessage) -> Result<String, String> {
      let id = format!("message-{}", self.messages.len());
      self.messages.push((channel_id.to_string(), message));
      Ok(id)
    }

    async fn create_thread(&mut self, channel_id: &str, message_id: &str, name: &str) -> Result<String, String> {
      let id = format!("thread-{}", self.threads.len());
      self
        .threads
        .push((channel_id.to_string(), message_id.to_string(), name.to_string()));
      Ok(id)
    }
  }

  #[tokio::test]
  async fn publishes_only_root_user_and_final_messages() {
    let fixture = TempDir::new().unwrap();
    let mut pet = Pet::new(
      FakeDiscord::default(),
      "channel".to_string(),
      fixture.path().join("state.json"),
    )
    .unwrap();

    pet
      .process(&message_event(
        Role::User,
        MessageDelivery::Unspecified,
        None,
        "build it",
      ))
      .await
      .unwrap();
    pet
      .process(&message_event(
        Role::Assistant,
        MessageDelivery::Commentary,
        None,
        "working",
      ))
      .await
      .unwrap();
    pet
      .process(&message_event(
        Role::Assistant,
        MessageDelivery::Final,
        Some("parent"),
        "child result",
      ))
      .await
      .unwrap();
    pet
      .process(&message_event(
        Role::Assistant,
        MessageDelivery::Final,
        None,
        "finished",
      ))
      .await
      .unwrap();

    assert_eq!(pet.api.threads.len(), 1);
    assert_eq!(pet.api.threads[0].2, "project · codex/session-");
    assert_eq!(pet.api.messages.len(), 2);
    assert_eq!(pet.api.messages[0].0, "channel");
    assert_eq!(pet.api.messages[0].1.embeds[0].title, "User");
    assert_eq!(pet.api.messages[1].0, "thread-0");
    assert_eq!(pet.api.messages[1].1.embeds[0].title, "Final");
  }

  #[tokio::test]
  async fn restores_thread_mapping_after_restart() {
    let fixture = TempDir::new().unwrap();
    let state_path = fixture.path().join("state.json");
    let mut first = Pet::new(FakeDiscord::default(), "channel".to_string(), state_path.clone()).unwrap();
    first
      .process(&message_event(Role::User, MessageDelivery::Unspecified, None, "first"))
      .await
      .unwrap();

    let mut restarted = Pet::new(FakeDiscord::default(), "channel".to_string(), state_path).unwrap();
    restarted
      .process(&message_event(Role::Assistant, MessageDelivery::Final, None, "done"))
      .await
      .unwrap();

    assert!(restarted.api.threads.is_empty());
    assert_eq!(restarted.api.messages[0].0, "thread-0");
  }

  #[test]
  fn splits_by_discord_utf16_length() {
    let chunks = split_message(&"😀".repeat(2_000), EMBED_DESCRIPTION_LIMIT);
    assert_eq!(chunks.len(), 2);
    assert!(
      chunks
        .iter()
        .all(|chunk| chunk.encode_utf16().count() <= EMBED_DESCRIPTION_LIMIT)
    );
  }

  fn message_event(role: Role, delivery: MessageDelivery, parent_session_id: Option<&str>, text: &str) -> RelayEvent {
    let mut project = ProjectContext::default();
    project.project_name = Some("project".to_string());
    RelayEvent {
      path: PathBuf::from("session.jsonl"),
      topic: "codex.session-id".to_string(),
      session: SessionContext {
        provider: Provider::Codex,
        session_id: "session-id".to_string(),
        parent_session_id: parent_session_id.map(str::to_string),
        agent_path: None,
        agent_nickname: None,
        agent_role: None,
        title: None,
        cwd: Some("/tmp/project".to_string()),
        started_at: None,
        project: Some(project),
      },
      event: AgentEvent::Message(MessageEvent {
        provider: Provider::Codex,
        session_id: Some("session-id".to_string()),
        message_id: None,
        parent_id: None,
        role,
        delivery,
        phase: Phase::Finished,
        text: text.to_string(),
        timestamp: None,
      }),
    }
  }
}
