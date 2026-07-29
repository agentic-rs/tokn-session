use std::io::{self, Write};
use std::path::Path;

use crate::{DiscordClient, DiscordConfig};

pub async fn login(config_path: &Path) -> Result<(), String> {
  print_setup_note(config_path);
  if config_path.exists() && !confirm("Replace the existing configuration? [y/N]: ")? {
    println!("Login cancelled; existing configuration was not changed.");
    return Ok(());
  }

  let bot_token = rpassword::prompt_password("Bot token (hidden): ")
    .map_err(|err| format!("failed to read Discord bot token: {err}"))?;
  let guild_id = prompt("Server ID: ")?;
  let channel_id = prompt("Channel ID: ")?;
  let config = DiscordConfig::new(
    bot_token.trim().to_string(),
    guild_id.trim().to_string(),
    channel_id.trim().to_string(),
  )?;

  println!("\nValidating the bot token and destination with Discord…");
  let mut api = DiscordClient::new(&config.bot_token)?;
  let username = api.validate_destination(&config.guild_id, &config.channel_id).await?;
  config.save(config_path)?;
  println!("Authenticated as @{username}.");
  println!("Saved protected configuration to {}.", config_path.display());
  Ok(())
}

fn print_setup_note(config_path: &Path) {
  println!(
    "\
Discord pet login

Before continuing:
  1. Bot token
     Discord Developer Portal → your application → Bot → Reset Token.
  2. Server and channel IDs
     Discord → User Settings → Advanced → enable Developer Mode.
     Right-click the server and target text channel, then choose Copy ID.
  3. Install the bot in that server and grant the target channel:
     View Channel, Send Messages, Create Public Threads,
     Send Messages in Threads, Read Message History, and Embed Links.

No privileged Discord intents are required.
The credentials will be validated before they are saved to:
  {}
",
    config_path.display()
  );
}

fn prompt(label: &str) -> Result<String, String> {
  print!("{label}");
  io::stdout()
    .flush()
    .map_err(|err| format!("failed to write prompt: {err}"))?;
  let mut value = String::new();
  io::stdin()
    .read_line(&mut value)
    .map_err(|err| format!("failed to read prompt: {err}"))?;
  Ok(value)
}

fn confirm(label: &str) -> Result<bool, String> {
  let answer = prompt(label)?;
  Ok(matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}
