#[path = "build/translation.rs"]
mod translation;

fn main() {
  tauri_build::build();
  translation::build();
}
