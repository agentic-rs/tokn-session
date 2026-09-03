use serde::{Deserialize, Serialize};
use tokn_session_relay::service_protocol::{DEFAULT_SERVICE_ENDPOINT, local_endpoint};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelayMode {
  #[default]
  Automatic,
  External,
  Local,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(from = "StoredSettings")]
pub struct RelaySettings {
  pub mode: RelayMode,
  /// Saved external endpoint, never overwritten by the child's ephemeral port.
  pub endpoint: String,
  pub include_native: bool,
}

#[derive(Deserialize)]
struct StoredSettings {
  mode: Option<RelayMode>,
  endpoint: Option<String>,
  enabled: Option<bool>,
  #[serde(default)]
  include_native: bool,
}

impl From<StoredSettings> for RelaySettings {
  fn from(saved: StoredSettings) -> Self {
    Self {
      mode: saved.mode.unwrap_or(match saved.enabled {
        Some(true) => RelayMode::External,
        Some(false) => RelayMode::Local,
        None => RelayMode::Automatic,
      }),
      endpoint: saved.endpoint.unwrap_or_else(|| DEFAULT_SERVICE_ENDPOINT.into()),
      include_native: saved.include_native,
    }
  }
}

impl Default for RelaySettings {
  fn default() -> Self {
    Self {
      mode: RelayMode::Automatic,
      endpoint: DEFAULT_SERVICE_ENDPOINT.into(),
      include_native: false,
    }
  }
}

impl RelaySettings {
  pub fn validate(&self) -> Result<(), String> {
    if self.mode == RelayMode::External {
      local_endpoint(&self.endpoint)?;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn migrates_explicit_legacy_choices_but_defaults_new_installs_to_automatic() {
    assert_eq!(RelaySettings::default().mode, RelayMode::Automatic);
    for (enabled, mode) in [(true, RelayMode::External), (false, RelayMode::Local)] {
      let settings: RelaySettings = serde_json::from_value(serde_json::json!({
        "endpoint": "tcp://127.0.0.1:9557", "enabled": enabled
      }))
      .unwrap();
      assert_eq!(settings.mode, mode);
      assert_eq!(settings.endpoint, "tcp://127.0.0.1:9557");
      assert!(!settings.include_native);
      let roundtrip: RelaySettings = serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
      assert_eq!(roundtrip, settings);
    }
  }

  #[test]
  fn local_mode_does_not_require_a_working_external_endpoint() {
    let mut settings = RelaySettings {
      mode: RelayMode::Local,
      endpoint: "invalid".into(),
      ..Default::default()
    };
    assert!(settings.validate().is_ok());
    settings.mode = RelayMode::External;
    assert!(settings.validate().is_err());
  }
}
