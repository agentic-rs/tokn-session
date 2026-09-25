use std::{
  collections::HashMap,
  fs,
  path::{Path, PathBuf},
};

/// Resolve a checkout or one of its subdirectories to the shared repository.
/// Worktree metadata is read directly; no Git subprocess runs during listing.
pub(crate) fn repository_root(cwd: &str) -> Option<PathBuf> {
  for directory in Path::new(cwd).ancestors() {
    let dot_git = directory.join(".git");
    let git_dir = if dot_git.is_dir() {
      dot_git
    } else if let Ok(contents) = fs::read_to_string(&dot_git) {
      directory.join(contents.trim().strip_prefix("gitdir:")?.trim())
    } else {
      continue;
    };
    let common = fs::read_to_string(git_dir.join("commondir"))
      .map(|value| git_dir.join(value.trim()))
      .unwrap_or(git_dir);
    let common = common.canonicalize().ok()?;
    return if common.file_name().is_some_and(|name| name == ".git") {
      common.parent().map(Path::to_path_buf)
    } else {
      Some(common)
    };
  }
  None
}

/// Registered worktrees can still identify historical sessions after their
/// checkout directory disappears. Never merge unrelated repositories by basename.
pub(crate) fn registered_worktrees(root: &Path) -> HashMap<String, String> {
  let mut aliases = HashMap::new();
  let Ok(entries) = fs::read_dir(root.join(".git/worktrees")) else {
    return aliases;
  };
  for entry in entries.flatten() {
    let Ok(gitfile) = fs::read_to_string(entry.path().join("gitdir")) else {
      continue;
    };
    if let Some(checkout) = Path::new(gitfile.trim()).parent() {
      aliases.insert(
        checkout.to_string_lossy().into_owned(),
        root.to_string_lossy().into_owned(),
      );
    }
  }
  aliases
}
