#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
  Dsh,
  Pi,
  Codex,
  OpenCode,
  ZCode,
}

impl Source {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Dsh => "dsh",
      Self::Pi => "pi",
      Self::Codex => "codex",
      Self::OpenCode => "opencode",
      Self::ZCode => "zcode",
    }
  }
}
