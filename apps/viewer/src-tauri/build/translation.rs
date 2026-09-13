use std::{env, path::PathBuf, process::Command};

pub fn build() {
  println!("cargo:rerun-if-changed=native/TranslationBridge.swift");
  println!("cargo:rerun-if-changed=build/translation.rs");
  println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
  println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
  println!("cargo:rerun-if-env-changed=SDKROOT");
  if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
    return;
  }

  let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
    "aarch64" => "arm64",
    "x86_64" => "x86_64",
    arch => panic!("Apple Translation does not support the macOS architecture {arch}"),
  };
  let mut deployment =
    env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| if arch == "arm64" { "11.0" } else { "10.13" }.into());
  // Tauri's default can still be 10.13, but arm64 macOS begins with Big Sur.
  if arch == "arm64"
    && deployment
      .split('.')
      .next()
      .and_then(|major| major.parse::<u32>().ok())
      .is_some_and(|major| major < 11)
  {
    deployment = "11.0".into();
  }
  let target = format!("{arch}-apple-macos{deployment}");
  let sdk = run(Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-path"]));
  let sdk = PathBuf::from(sdk.trim());
  assert!(
    sdk.join("System/Library/Frameworks/Translation.framework").exists(),
    "Building the Mac viewer requires Xcode 16 or newer with the macOS 15 SDK for Apple Translation."
  );
  let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
  let archive = output.join("libToknTranslation.a");
  run(
    Command::new("xcrun")
      .args([
        "--sdk",
        "macosx",
        "swiftc",
        "-parse-as-library",
        "-emit-library",
        "-static",
        "-O",
        "-whole-module-optimization",
        "-swift-version",
        "6",
        "-module-name",
        "ToknTranslation",
        "-target",
        &target,
      ])
      .arg("-sdk")
      .arg(&sdk)
      .arg("-module-cache-path")
      .arg(output.join("swift-module-cache"))
      // The framework only exists on macOS 15+. Weak linking plus Swift's
      // #available guards keeps the rest of the viewer usable on older Macs.
      .args(["-Xfrontend", "-disable-autolink-framework", "-Xfrontend", "Translation"])
      .arg("native/TranslationBridge.swift")
      .arg("-o")
      .arg(archive),
  );
  let info = run(Command::new("xcrun").args(["--sdk", "macosx", "swiftc", "-print-target-info", "-target", &target]));
  let info: serde_json::Value = serde_json::from_str(&info).expect("Swift target information must be JSON");
  for path in info["paths"]["runtimeLibraryPaths"]
    .as_array()
    .expect("Swift runtime paths")
  {
    println!(
      "cargo:rustc-link-search=native={}",
      path.as_str().expect("Swift runtime path")
    );
  }
  println!("cargo:rustc-link-search=native={}", sdk.join("usr/lib/swift").display());
  println!("cargo:rustc-link-search=native={}", output.display());
  println!("cargo:rustc-link-lib=static=ToknTranslation");
  println!("cargo:rustc-link-arg=-Wl,-weak_framework,Translation");
  // Swift's back-deployed concurrency import uses @rpath. A link-search path
  // only helps the linker; dyld also needs the system Swift runtime location.
  // Without this, the weak MainActor symbols resolve to NULL on first use.
  println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}

fn run(command: &mut Command) -> String {
  let output = command.output().unwrap_or_else(|error| {
    panic!("Could not run {command:?}: {error}. Install Xcode 16 or newer to build Apple Translation.")
  });
  assert!(
    output.status.success(),
    "Command {command:?} failed:\n{}",
    String::from_utf8_lossy(&output.stderr)
  );
  String::from_utf8(output.stdout).expect("Swift tool output must be UTF-8")
}
