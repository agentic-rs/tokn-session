use std::path::{Path, PathBuf};

use crate::normalize::OpenCodeNormalizer;
use crate::row::{OpenCodeMessageRow, OpenCodePartRow, OpenCodeSessionRow};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tokn_opencode_protocol::v1::{MessageData, PartData, SessionModel};
use tokn_session_core::{LoadedSession, SessionHistoryStatus, SessionRef};

pub struct OpenCodeSessionSource {
  session_dir: Option<PathBuf>,
}

impl OpenCodeSessionSource {
  pub fn new(session_dir: Option<PathBuf>) -> Self {
    Self { session_dir }
  }

  pub fn list_sessions(&self) -> Result<Vec<SessionRef>, String> {
    let connection = self.connect()?;
    let mut statement = connection
      .prepare(
        "select id, parent_id, directory, model, time_created, time_updated
         from session
         order by time_created desc, id desc",
      )
      .map_err(|err| format!("failed to prepare opencode session query: {err}"))?;
    let rows = statement
      .query_map([], |row| {
        let model: Option<String> = row.get(3)?;
        Ok(OpenCodeSessionRow {
          id: row.get(0)?,
          parent_id: row.get(1)?,
          directory: row.get(2)?,
          model: parse_optional_model(model),
          time_created: row.get(4)?,
          time_updated: row.get(5)?,
        })
      })
      .map_err(|err| format!("failed to query opencode sessions: {err}"))?;

    let mut sessions = Vec::new();
    for row in rows {
      let row = row.map_err(|err| format!("failed to read opencode session row: {err}"))?;
      let message_count = message_count(&connection, &row.id)?;
      sessions.push(SessionRef {
        id: row.id,
        parent_session_id: row.parent_id,
        agent_path: None,
        agent_nickname: None,
        agent_role: None,
        path: self.database_path()?,
        cwd: row.directory,
        timestamp: timestamp(row.time_updated.or(row.time_created)),
        message_count,
      });
    }
    Ok(sessions)
  }

  pub fn load_session(&self, id_or_path: &str) -> Result<LoadedSession, String> {
    let (database_path, session_id) = self.resolve_session(id_or_path)?;
    self.load_session_from_database(database_path, &session_id)
  }

  pub fn load_session_exact(&self, session_id: &str) -> Result<LoadedSession, String> {
    self.load_session_from_database(self.database_path()?, session_id)
  }

  fn load_session_from_database(&self, database_path: PathBuf, session_id: &str) -> Result<LoadedSession, String> {
    let connection = connect_database(&database_path)?;
    let session = load_session_row(&connection, session_id)?
      .ok_or_else(|| format!("no opencode session found for `{session_id}`"))?;
    let reference = SessionRef {
      id: session.id.clone(),
      parent_session_id: session.parent_id.clone(),
      agent_path: None,
      agent_nickname: None,
      agent_role: None,
      path: database_path,
      cwd: session.directory.clone(),
      timestamp: timestamp(session.time_updated.or(session.time_created)),
      message_count: message_count(&connection, &session.id)?,
    };

    let mut normalizer = OpenCodeNormalizer::new(session.id.clone());
    let mut events = normalizer.normalize_session(&session);
    for message in load_messages(&connection, &session.id)? {
      events.extend(normalizer.normalize_message(message));
    }

    Ok(LoadedSession {
      reference,
      events,
      history_status: SessionHistoryStatus::Complete,
    })
  }

  fn resolve_session(&self, id_or_path: &str) -> Result<(PathBuf, String), String> {
    let candidate = PathBuf::from(id_or_path);
    if candidate.exists() {
      return Err(
        "opencode sessions are stored in sqlite; pass a session id and use --session-dir for the database".to_string(),
      );
    }

    let matches: Vec<_> = self
      .list_sessions()?
      .into_iter()
      .filter(|session| session.id == id_or_path || session.id.starts_with(id_or_path))
      .collect();

    match matches.as_slice() {
      [session] => Ok((session.path.clone(), session.id.clone())),
      [] => Err(format!("no opencode session found for `{id_or_path}`")),
      _ => Err(format!("multiple opencode sessions match `{id_or_path}`")),
    }
  }

  fn connect(&self) -> Result<Connection, String> {
    connect_database(&self.database_path()?)
  }

  fn database_path(&self) -> Result<PathBuf, String> {
    if let Some(path) = &self.session_dir {
      if path.is_dir() {
        return Ok(path.join("opencode.db"));
      }
      return Ok(path.clone());
    }

    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?;
    Ok(
      PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db"),
    )
  }
}

fn connect_database(path: &Path) -> Result<Connection, String> {
  let uri = format!("file:{}?mode=ro&immutable=1", sqlite_uri_path(path));
  Connection::open_with_flags(&uri, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)
    .map_err(|err| format!("failed to open opencode database {}: {err}", path.display()))
}

fn sqlite_uri_path(path: &Path) -> String {
  path
    .to_string_lossy()
    .chars()
    .flat_map(|value| match value {
      ' ' => "%20".chars().collect::<Vec<_>>(),
      '#' => "%23".chars().collect::<Vec<_>>(),
      '?' => "%3f".chars().collect::<Vec<_>>(),
      '%' => "%25".chars().collect::<Vec<_>>(),
      value => vec![value],
    })
    .collect()
}

fn load_session_row(connection: &Connection, session_id: &str) -> Result<Option<OpenCodeSessionRow>, String> {
  connection
    .query_row(
      "select id, parent_id, directory, model, time_created, time_updated from session where id = ?1",
      params![session_id],
      |row| {
        let model: Option<String> = row.get(3)?;
        Ok(OpenCodeSessionRow {
          id: row.get(0)?,
          parent_id: row.get(1)?,
          directory: row.get(2)?,
          model: parse_optional_model(model),
          time_created: row.get(4)?,
          time_updated: row.get(5)?,
        })
      },
    )
    .optional()
    .map_err(|err| format!("failed to load opencode session `{session_id}`: {err}"))
}

fn load_messages(connection: &Connection, session_id: &str) -> Result<Vec<OpenCodeMessageRow>, String> {
  let mut statement = connection
    .prepare(
      "select id, time_created, data
       from message
       where session_id = ?1
       order by time_created asc, id asc",
    )
    .map_err(|err| format!("failed to prepare opencode message query: {err}"))?;
  let rows = statement
    .query_map(params![session_id], |row| {
      let data: String = row.get(2)?;
      Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?, data))
    })
    .map_err(|err| format!("failed to query opencode messages: {err}"))?;

  let mut messages = Vec::new();
  for row in rows {
    let (id, time_created, data) = row.map_err(|err| format!("failed to read opencode message row: {err}"))?;
    let data: MessageData =
      serde_json::from_str(&data).map_err(|err| format!("invalid opencode message `{id}`: {err}"))?;
    let parts = load_parts(connection, session_id, &id)?;
    messages.push(OpenCodeMessageRow {
      id,
      time_created,
      data,
      parts,
    });
  }
  Ok(messages)
}

fn load_parts(connection: &Connection, session_id: &str, message_id: &str) -> Result<Vec<OpenCodePartRow>, String> {
  let mut statement = connection
    .prepare(
      "select id, time_created, data
       from part
       where session_id = ?1 and message_id = ?2
       order by time_created asc, id asc",
    )
    .map_err(|err| format!("failed to prepare opencode part query: {err}"))?;
  let rows = statement
    .query_map(params![session_id, message_id], |row| {
      let data: String = row.get(2)?;
      Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?, data))
    })
    .map_err(|err| format!("failed to query opencode parts: {err}"))?;

  let mut parts = Vec::new();
  for row in rows {
    let (id, time_created, data) = row.map_err(|err| format!("failed to read opencode part row: {err}"))?;
    let data: PartData = serde_json::from_str(&data).map_err(|err| format!("invalid opencode part `{id}`: {err}"))?;
    parts.push(OpenCodePartRow { id, time_created, data });
  }
  Ok(parts)
}

fn message_count(connection: &Connection, session_id: &str) -> Result<usize, String> {
  connection
    .query_row(
      "select count(*) from message where session_id = ?1",
      params![session_id],
      |row| row.get::<_, i64>(0),
    )
    .map(|count| count as usize)
    .map_err(|err| format!("failed to count opencode messages for `{session_id}`: {err}"))
}

fn parse_optional_model(value: Option<String>) -> Option<SessionModel> {
  value.and_then(|value| serde_json::from_str(&value).ok())
}

fn timestamp(value: Option<i64>) -> Option<String> {
  value.map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
  use rusqlite::{Connection, params};
  use tokn_opencode_protocol::v1::{MessageItem, PartItem};

  use super::load_messages;

  #[test]
  fn loads_unknown_payloads_without_aborting_the_session() {
    let connection = Connection::open_in_memory().expect("database should open");
    connection
      .execute_batch(
        "create table message (
           id text primary key,
           session_id text not null,
           time_created integer,
           data text not null
         );
         create table part (
           id text primary key,
           message_id text not null,
           session_id text not null,
           time_created integer,
           data text not null
         );",
      )
      .expect("fixture schema should be created");
    connection
      .execute(
        "insert into message (id, session_id, time_created, data)
         values (?1, ?2, ?3, ?4)",
        params![
          "msg_1",
          "ses_1",
          1_i64,
          r#"{"role":"future-role","payload":{"answer":42}}"#
        ],
      )
      .expect("message fixture should insert");
    connection
      .execute(
        "insert into part (id, message_id, session_id, time_created, data)
         values (?1, ?2, ?3, ?4, ?5)",
        params![
          "prt_1",
          "msg_1",
          "ses_1",
          2_i64,
          r#"{"type":"future-part","answer":42}"#
        ],
      )
      .expect("part fixture should insert");

    let messages = load_messages(&connection, "ses_1").expect("unknown payloads should remain loadable");
    assert_eq!(messages.len(), 1);
    assert!(matches!(
      messages[0].data.item(),
      MessageItem::Unknown(item) if item.native_type.as_deref() == Some("future-role")
    ));
    assert_eq!(messages[0].parts[0].id, "prt_1");
    assert!(matches!(
      messages[0].parts[0].data.item(),
      PartItem::Unknown(item) if item.native_type.as_deref() == Some("future-part")
    ));
  }
}
