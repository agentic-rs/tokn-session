mod args;
mod browser;

use args::{Command, Format};
use browser::{browse_session, browse_sessions};
use tokn_session_client::AgentClient;
use tokn_session_render::{render_agent_jsonl, render_pretty, render_session_list};

fn main() {
  if let Err(err) = run() {
    eprintln!("error: {err}");
    std::process::exit(1);
  }
}

fn run() -> Result<(), String> {
  let cli = args::parse(std::env::args().skip(1).collect())?;

  match cli.command {
    Command::List {
      source,
      session_dir,
      limit,
    } => {
      let mut sessions = AgentClient::list_sessions(source, session_dir)?;
      if limit > 0 {
        sessions.truncate(limit);
      }
      print!("{}", render_session_list(&sessions));
      Ok(())
    }
    Command::Show {
      source,
      session,
      format,
      session_dir,
    } => {
      let loaded = AgentClient::load_session(source, session_dir, &session)?;
      match format {
        Format::Pretty => print!("{}", render_pretty(&loaded)),
        Format::Jsonl => print!("{}", render_agent_jsonl(&loaded.events)?),
      }
      Ok(())
    }
    Command::Browse {
      source,
      session,
      session_dir,
    } => {
      if let Some(session) = session {
        let loaded = AgentClient::load_session(source, session_dir, &session)?;
        return browse_session(&loaded);
      }

      let sessions = AgentClient::list_sessions(source, session_dir.clone())?;
      browse_sessions(sessions, |session| {
        AgentClient::load_session(source, session_dir.clone(), session)
      })
    }
  }
}
