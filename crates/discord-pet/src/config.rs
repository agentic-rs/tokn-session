use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscordConfig {
  pub bot_token: String,
  pub guild_id: String,
  pub channel_id: String,
}

impl DiscordConfig {
  pub fn load(path: &Path) -> Result<Self, String> {
    let contents =
      fs::read_to_string(path).map_err(|err| format!("failed to read Discord pet config {}: {err}", path.display()))?;
    let config: Self = serde_yaml_ng::from_str(&contents)
      .map_err(|err| format!("failed to parse Discord pet config {}: {err}", path.display()))?;
    config.validate()?;
    Ok(config)
  }

  fn validate(&self) -> Result<(), String> {
    if self.bot_token.trim().is_empty() {
      return Err("Discord pet config `bot_token` must not be empty".to_string());
    }
    validate_snowflake("guild_id", &self.guild_id)?;
    validate_snowflake("channel_id", &self.channel_id)
  }
}

fn validate_snowflake(field: &str, value: &str) -> Result<(), String> {
  if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
    return Err(format!("Discord pet config `{field}` must be a numeric Discord ID"));
  }
  Ok(())
}

pub fn default_config_path() -> Result<PathBuf, String> {
  std::env::var_os("HOME")
    .map(PathBuf::from)
    .map(|home| home.join(".tokn/pet/discord.yaml"))
    .ok_or_else(|| "HOME is not set; pass `--config <path>`".to_string())
}

pub fn state_path(config_path: &Path) -> PathBuf {
  config_path.with_file_name("discord-state.json")
}

#[cfg(unix)]
pub fn permissions_warning(path: &Path) -> Option<String> {
  use std::os::unix::fs::PermissionsExt;

  let mode = fs::metadata(path).ok()?.permissions().mode();
  (mode & 0o077 != 0).then(|| {
    format!(
      "{} is readable by other users; protect the bot token with `chmod 600 {}`",
      path.display(),
      path.display()
    )
  })
}

#[cfg(not(unix))]
pub fn permissions_warning(_path: &Path) -> Option<String> {
  None
}

#[cfg(test)]
mod tests {
  use std::fs;

  use tempfile::TempDir;

  use super::{DiscordConfig, state_path};

  #[test]
  fn loads_strict_valid_config() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("discord.yaml");
    fs::write(
      &path,
      "bot_token: secret\nguild_id: \"123456789\"\nchannel_id: \"987654321\"\n",
    )
    .unwrap();

    let config = DiscordConfig::load(&path).unwrap();
    assert_eq!(config.guild_id, "123456789");
    assert_eq!(config.channel_id, "987654321");
    assert_eq!(state_path(&path), fixture.path().join("discord-state.json"));
  }

  #[test]
  fn rejects_unknown_fields_and_non_numeric_ids() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("discord.yaml");
    fs::write(
      &path,
      "bot_token: secret\nguild_id: nope\nchannel_id: \"987\"\nextra: true\n",
    )
    .unwrap();

    assert!(DiscordConfig::load(&path).is_err());
  }
}
