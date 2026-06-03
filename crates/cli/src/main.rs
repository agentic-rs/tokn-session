mod args;
mod render;

use args::{Command, Format, SessionsCommand};
use render::{render_agent_jsonl, render_pretty, render_session_list};
use tokn_agent_client::AgentClient;

fn main() {
  if let Err(err) = run() {
    eprintln!("error: {err}");
    std::process::exit(1);
  }
}

fn run() -> Result<(), String> {
  let cli = args::parse(std::env::args().skip(1).collect())?;

  match cli.command {
    Command::Sessions(SessionsCommand::List {
      source,
      session_dir,
      limit,
    }) => {
      let mut sessions = AgentClient::list_sessions(source, session_dir)?;
      if limit > 0 {
        sessions.truncate(limit);
      }
      print!("{}", render_session_list(&sessions));
      Ok(())
    }
    Command::Sessions(SessionsCommand::Show {
      source,
      session,
      format,
      session_dir,
    }) => {
      let loaded = AgentClient::load_session(source, session_dir, &session)?;
      match format {
        Format::Pretty => print!("{}", render_pretty(&loaded)),
        Format::Jsonl => print!("{}", render_agent_jsonl(&loaded.events)?),
      }
      Ok(())
    }
  }
}
