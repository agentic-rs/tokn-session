mod agent_event;
mod cli;
mod pi;
mod render;

use std::path::PathBuf;

use cli::{Command, Format, SessionsCommand, Source};
use pi::session_source::PiSessionSource;
use render::{render_agent_jsonl, render_pretty, render_session_list};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = cli::parse(std::env::args().skip(1).collect())?;

    match cli.command {
        Command::Sessions(SessionsCommand::List {
            source,
            session_dir,
            limit,
        }) => {
            let mut sessions = session_source(source, session_dir)?.list_sessions()?;
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
            let source = session_source(source, session_dir)?;
            let loaded = source.load_session(&session)?;
            match format {
                Format::Pretty => print!("{}", render_pretty(&loaded)),
                Format::Jsonl => print!("{}", render_agent_jsonl(&loaded.events)?),
            }
            Ok(())
        }
    }
}

fn session_source(source: Source, session_dir: Option<PathBuf>) -> Result<PiSessionSource, String> {
    match source {
        Source::Pi => Ok(PiSessionSource::new(session_dir)),
        Source::Codex => Err("codex sessions are not implemented yet".to_string()),
        Source::OpenCode => Err("opencode sessions are not implemented yet".to_string()),
    }
}
