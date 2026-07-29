use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscordConfig {
  pub bot_token: String,
  pub guild_id: String,
  pub channel_id: String,
}

impl DiscordConfig {
  pub fn new(bot_token: String, guild_id: String, channel_id: String) -> Result<Self, String> {
    let config = Self {
      bot_token,
      guild_id,
      channel_id,
    };
    config.validate()?;
    Ok(config)
  }

  pub fn load(path: &Path) -> Result<Self, String> {
    let contents =
      fs::read_to_string(path).map_err(|err| format!("failed to read Discord pet config {}: {err}", path.display()))?;
    let config: Self = serde_yaml_ng::from_str(&contents)
      .map_err(|err| format!("failed to parse Discord pet config {}: {err}", path.display()))?;
    config.validate()?;
    Ok(config)
  }

  pub fn save(&self, path: &Path) -> Result<(), String> {
    self.validate()?;
    let parent = path
      .parent()
      .filter(|parent| !parent.as_os_str().is_empty())
      .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|err| {
      format!(
        "failed to create Discord pet config directory {}: {err}",
        parent.display()
      )
    })?;
    let temporary = path.with_extension("yaml.tmp");
    let yaml =
      serde_yaml_ng::to_string(self).map_err(|err| format!("failed to serialize Discord pet config: {err}"))?;
    write_private_file(&temporary, yaml.as_bytes())?;
    fs::rename(&temporary, path)
      .map_err(|err| format!("failed to replace Discord pet config {}: {err}", path.display()))?;
    set_private_permissions(path)
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

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
  let mut options = OpenOptions::new();
  options.create(true).truncate(true).write(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
  }
  let mut file = options
    .open(path)
    .map_err(|err| format!("failed to create Discord pet config {}: {err}", path.display()))?;
  file
    .write_all(contents)
    .and_then(|_| file.sync_all())
    .map_err(|err| format!("failed to write Discord pet config {}: {err}", path.display()))
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), String> {
  use std::os::unix::fs::PermissionsExt;

  fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    .map_err(|err| format!("failed to protect Discord pet config {}: {err}", path.display()))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), String> {
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

  #[test]
  fn saves_and_reloads_config() {
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("nested/discord.yaml");
    let config = DiscordConfig::new("secret".to_string(), "123456789".to_string(), "987654321".to_string()).unwrap();

    config.save(&path).unwrap();
    let loaded = DiscordConfig::load(&path).unwrap();
    assert_eq!(loaded.bot_token, "secret");
    assert_eq!(loaded.guild_id, "123456789");
    assert_eq!(loaded.channel_id, "987654321");
  }

  #[cfg(unix)]
  #[test]
  fn saves_config_with_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("discord.yaml");
    fs::write(&path, "old config").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let config = DiscordConfig::new("secret".to_string(), "123".to_string(), "987".to_string()).unwrap();

    config.save(&path).unwrap();

    assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
  }
}
