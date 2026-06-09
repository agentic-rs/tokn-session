#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
  Pi,
  Codex,
  OpenCode,
}

impl Source {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Pi => "pi",
      Self::Codex => "codex",
      Self::OpenCode => "opencode",
    }
  }
}
