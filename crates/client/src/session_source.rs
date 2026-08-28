use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tokn_session_codex::CodexSessionSource;
use tokn_session_core::{LoadedSession, LoadedSessionTree, SessionRef};
use tokn_session_dsh::DshSessionSource;
use tokn_session_opencode::OpenCodeSessionSource;
use tokn_session_pi::PiSessionSource;

use crate::Source;

pub(crate) enum SessionSourceClient {
  Dsh(DshSessionSource),
  Codex(CodexSessionSource),
  OpenCode(OpenCodeSessionSource),
  Pi(PiSessionSource),
}

impl SessionSourceClient {
  pub(crate) fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    match self {
      Self::Dsh(source) => source.list_sessions(),
      Self::Codex(source) => source.list_sessions(),
      Self::OpenCode(source) => source.list_sessions(),
      Self::Pi(source) => source.list_sessions(),
    }
  }

  pub(crate) fn list_session_relations(&self) -> Result<Vec<SessionRef>, String> {
    match self {
      Self::Dsh(source) => source.list_session_relations(),
      Self::Codex(source) => source.list_session_relations(),
      Self::OpenCode(source) => source.list_sessions(),
      Self::Pi(source) => source.list_session_relations(),
    }
  }

  pub(crate) fn load_session(&self, session: &str) -> Result<LoadedSession, String> {
    match self {
      Self::Dsh(source) => source.load_session(session),
      Self::Codex(source) => source.load_session(session),
      Self::OpenCode(source) => source.load_session(session),
      Self::Pi(source) => source.load_session(session),
    }
  }

  pub(crate) fn load_session_path(&self, path: &Path) -> Result<LoadedSession, String> {
    match self {
      Self::Dsh(source) => source.load_session_path(path),
      Self::Codex(source) => source.load_session_path(path),
      Self::OpenCode(_) => {
        Err("opencode sessions are stored in sqlite; pass a session id and use --session-dir for the database".into())
      }
      Self::Pi(source) => source.load_session_path(path),
    }
  }

  pub(crate) fn load_session_tree(&self, session: &str) -> Result<LoadedSessionTree, String> {
    let root = self.load_session(session)?;
    self.load_session_tree_from(root, self.list_session_relations()?)
  }

  pub(crate) fn load_session_tree_from(
    &self,
    root: LoadedSession,
    mut references: Vec<SessionRef>,
  ) -> Result<LoadedSessionTree, String> {
    references.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.path.cmp(&a.path)));
    let mut canonical_ids = HashSet::new();
    references.retain(|reference| canonical_ids.insert(reference.id.clone()));
    references.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.path.cmp(&b.path)));

    let mut children_by_parent = HashMap::<String, Vec<SessionRef>>::new();
    for reference in references {
      if let Some(parent_session_id) = reference.parent_session_id.clone() {
        children_by_parent.entry(parent_session_id).or_default().push(reference);
      }
    }

    let mut visited = HashSet::new();
    visited.insert(root.reference.id.clone());
    self.load_descendants(root, &mut children_by_parent, &mut visited)
  }

  fn load_descendants(
    &self,
    session: LoadedSession,
    children_by_parent: &mut HashMap<String, Vec<SessionRef>>,
    visited: &mut HashSet<String>,
  ) -> Result<LoadedSessionTree, String> {
    let mut children = Vec::new();
    for child in children_by_parent.remove(&session.reference.id).unwrap_or_default() {
      if !visited.insert(child.id.clone()) {
        continue;
      }
      let loaded = self
        .load_reference(&child)
        .map_err(|err| format!("failed to load child session `{}`: {err}", child.id))?;
      children.push(self.load_descendants(loaded, children_by_parent, visited)?);
    }

    Ok(LoadedSessionTree { session, children })
  }

  fn load_reference(&self, reference: &SessionRef) -> Result<LoadedSession, String> {
    match self {
      Self::Dsh(source) => source.load_session_path(&reference.path),
      Self::Codex(source) => source.load_session_path(&reference.path),
      Self::OpenCode(source) => source.load_session_exact(&reference.id),
      Self::Pi(source) => source.load_session_path(&reference.path),
    }
  }
}

pub(crate) fn session_source(source: Source, session_dir: Option<PathBuf>) -> Result<SessionSourceClient, String> {
  match source {
    Source::Dsh => Ok(SessionSourceClient::Dsh(DshSessionSource::new(session_dir))),
    Source::Pi => Ok(SessionSourceClient::Pi(PiSessionSource::new(session_dir))),
    Source::Codex => Ok(SessionSourceClient::Codex(CodexSessionSource::new(session_dir))),
    Source::OpenCode => Ok(SessionSourceClient::OpenCode(OpenCodeSessionSource::new(session_dir))),
  }
}
