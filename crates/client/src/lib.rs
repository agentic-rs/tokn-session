mod print;
mod session_source;
mod source;

use std::path::{Path, PathBuf};

use tokn_session_core::{LoadedSession, LoadedSessionTree, SessionRef};

pub use print::{AppendAction, AppendSessionRequest, CreateSessionRequest};
pub use source::Source;

pub struct AgentClient;

impl AgentClient {
  pub fn list_sessions(source: Source, session_dir: Option<PathBuf>) -> Result<Vec<SessionRef>, String> {
    session_source::session_source(source, session_dir)?.list_sessions()
  }

  pub fn load_session(source: Source, session_dir: Option<PathBuf>, session: &str) -> Result<LoadedSession, String> {
    session_source::session_source(source, session_dir)?.load_session(session)
  }

  pub fn load_session_tree(
    source: Source,
    session_dir: Option<PathBuf>,
    session: &str,
  ) -> Result<LoadedSessionTree, String> {
    let plan = tree_discovery_plan(source, session_dir, session);
    let client = session_source::session_source(source, plan.session_dir)?;
    let Some(path) = plan.explicit_path else {
      return client.load_session_tree(session);
    };

    let root = client.load_session_path(&path)?;
    let mut references = client.list_session_relations()?;
    if let Some(supplemental_dir) = plan.supplemental_dir {
      references.extend(session_source::session_source(source, Some(supplemental_dir))?.list_session_relations()?);
    }
    client.load_session_tree_from(root, references)
  }

  pub fn create_session(request: CreateSessionRequest) -> Result<(), String> {
    print::create_session(request)
  }

  pub fn append_session(request: AppendSessionRequest) -> Result<(), String> {
    print::append_session(request)
  }
}

#[derive(Debug, Eq, PartialEq)]
struct TreeDiscoveryPlan {
  session_dir: Option<PathBuf>,
  supplemental_dir: Option<PathBuf>,
  explicit_path: Option<PathBuf>,
}

fn tree_discovery_plan(source: Source, session_dir: Option<PathBuf>, session: &str) -> TreeDiscoveryPlan {
  if source == Source::OpenCode {
    return TreeDiscoveryPlan {
      session_dir,
      supplemental_dir: None,
      explicit_path: None,
    };
  }

  let session_path = Path::new(session);
  if !session_path.is_file() {
    return TreeDiscoveryPlan {
      session_dir,
      supplemental_dir: None,
      explicit_path: None,
    };
  }

  let supplemental_dir = session_dir.is_none().then(|| {
    session_path
      .parent()
      .filter(|path| !path.as_os_str().is_empty())
      .unwrap_or_else(|| Path::new("."))
      .to_path_buf()
  });
  TreeDiscoveryPlan {
    session_dir,
    supplemental_dir,
    explicit_path: Some(session_path.to_path_buf()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn loads_dsh_tree_without_forks_or_inherited_parent_messages() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = include_str!("../../dsh/fixtures/basic/session.jsonl");
    for (id, metadata) in [
      ("root", ""),
      (
        "child",
        ",\"parentSession\":\"root\",\"origin\":\"subagent\",\"seedLength\":2",
      ),
      ("fork", ",\"parentSession\":\"root\",\"seedLength\":2"),
    ] {
      let folder = dir.path().join(id);
      std::fs::create_dir(&folder).unwrap();
      let content = fixture
        .replace("dsh-fixture", id)
        .replace("\"delegationDepth\":0", &format!("\"delegationDepth\":0{metadata}"));
      std::fs::write(folder.join("session.jsonl"), content).unwrap();
    }
    let path = dir.path().join("root/session.jsonl");
    let tree = AgentClient::load_session_tree(Source::Dsh, Some(dir.path().into()), path.to_str().unwrap()).unwrap();
    assert_eq!(tree.session.reference.id, "root");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].session.reference.id, "child");
    assert_eq!(tree.children[0].session.reference.message_count, 3);
  }

  #[test]
  fn loads_codex_session_tree_recursively() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codex/fixtures");
    let tree =
      AgentClient::load_session_tree(Source::Codex, Some(fixtures), "tree-root").expect("fixture tree should load");

    assert_eq!(tree.session.reference.id, "tree-root");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].session.reference.id, "tree-child");
    assert_eq!(
      tree.children[0]
        .session
        .reference
        .path
        .file_name()
        .and_then(|name| name.to_str()),
      Some("tree_child.jsonl")
    );
    assert_eq!(tree.children[0].children.len(), 1);
    assert_eq!(tree.children[0].children[0].session.reference.id, "tree-grandchild");
  }

  #[test]
  fn discovers_codex_siblings_for_an_explicit_session_path() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codex/fixtures");
    let root = fixtures.join("tree_root.jsonl");
    let tree = AgentClient::load_session_tree(
      Source::Codex,
      Some(fixtures),
      root.to_str().expect("fixture path should be utf-8"),
    )
    .expect("fixture tree should load");

    assert_eq!(tree.session.reference.id, "tree-root");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].session.reference.id, "tree-child");
  }

  #[test]
  fn loads_pi_session_tree_from_parent_paths() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pi/fixtures");
    let tree =
      AgentClient::load_session_tree(Source::Pi, Some(fixtures), "pi-tree-root").expect("fixture tree should load");

    assert_eq!(tree.session.reference.id, "pi-tree-root");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].session.reference.id, "pi-tree-child");
  }

  #[test]
  fn discovers_pi_siblings_for_an_explicit_session_path() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pi/fixtures");
    let root = fixtures.join("tree_parent.jsonl");
    let tree = AgentClient::load_session_tree(
      Source::Pi,
      Some(fixtures),
      root.to_str().expect("fixture path should be utf-8"),
    )
    .expect("fixture tree should load");

    assert_eq!(tree.session.reference.id, "pi-tree-root");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].session.reference.id, "pi-tree-child");
  }

  #[test]
  fn explicit_file_keeps_default_discovery_and_adds_its_directory() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codex/fixtures/tree_root.jsonl");
    let plan = tree_discovery_plan(
      Source::Codex,
      None,
      root.to_str().expect("fixture path should be utf-8"),
    );

    assert_eq!(plan.session_dir, None);
    assert_eq!(plan.supplemental_dir, root.parent().map(Path::to_path_buf));
    assert_eq!(plan.explicit_path.as_deref(), Some(root.as_path()));
  }

  #[test]
  fn explicit_codex_path_finds_descendants_in_other_discovery_subdirectories() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codex/fixtures/tree_cross_day");
    let root = fixtures.join("day_one/root.jsonl");
    let tree = AgentClient::load_session_tree(
      Source::Codex,
      Some(fixtures),
      root.to_str().expect("fixture path should be utf-8"),
    )
    .expect("cross-directory fixture tree should load");

    assert_eq!(tree.session.reference.id, "cross-day-root");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].session.reference.id, "cross-day-child");
  }
}
