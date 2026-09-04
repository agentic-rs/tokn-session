//! SQLite-only queries used to admit and locate on-demand snapshots.
use crate::{
  model::{SessionLocator, ViewerProvider},
  service::{index_catalog_source_key, index_source_key_for_path, indexed_session_header},
  service_protocol::CatalogEntry,
};
use tokn_session_index::{IndexedSession, SessionIndex, SessionKey};

pub(crate) fn snapshot_entries(index: &SessionIndex) -> Result<Vec<CatalogEntry>, String> {
  let mut ready = std::collections::BTreeSet::new();
  for provider in ViewerProvider::ALL {
    if index
      .source_state(&index_catalog_source_key(provider))
      .map_err(|e| e.to_string())?
      .is_some()
    {
      ready.insert(provider.as_str());
    }
  }
  let sessions = index.list_present_sessions().map_err(|e| e.to_string())?;
  Ok(
    sessions
      .iter()
      .filter(|session| ready.contains(session.key.provider.as_str()))
      .filter_map(|session| snapshot_entry_from_row(session).ok())
      .collect(),
  )
}

pub(crate) fn snapshot_entry(index: &SessionIndex, locator: &SessionLocator) -> Result<Option<CatalogEntry>, String> {
  // A partial first catalog does not admit requests until its sentinel commits.
  if index
    .source_state(&index_catalog_source_key(locator.provider))
    .map_err(|e| e.to_string())?
    .is_none()
  {
    return Ok(None);
  }
  let source = index_source_key_for_path(locator.provider, &locator.source_path)?;
  let key = SessionKey::new(source.provider, source.source_key, &locator.session_id);
  index
    .session(&key)
    .map_err(|e| e.to_string())?
    .filter(|session| session.present)
    .map(|session| snapshot_entry_from_row(&session))
    .transpose()
}

pub(crate) fn snapshot_entry_for_key(index: &SessionIndex, key: &str) -> Result<Option<CatalogEntry>, String> {
  let (provider, source_path, session_id) =
    serde_json::from_str(key).map_err(|e| format!("Invalid snapshot key: {e}"))?;
  let entry = snapshot_entry(
    index,
    &SessionLocator {
      version: 1,
      provider,
      source_path,
      session_id,
    },
  )?;
  Ok(entry.filter(|entry| entry.key == key))
}

fn snapshot_entry_from_row(session: &IndexedSession) -> Result<CatalogEntry, String> {
  let header = indexed_session_header(session)?;
  let provider = serde_json::from_value::<tokn_session_core::Provider>(serde_json::json!(session.key.provider))
    .map_err(|e| e.to_string())?;
  Ok(CatalogEntry {
    key: serde_json::to_string(&(provider, &header.path, &header.id)).map_err(|e| e.to_string())?,
    provider,
    header,
  })
}
