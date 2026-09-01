//! A compact, app-owned SQLite index for historical agent sessions.
//!
//! The index deliberately stores only source checkpoints, session identity, and
//! bounded caller-supplied metadata. It never stores normalized events, native
//! provider payloads, reasoning, tool inputs, or tool outputs. `title` and a
//! bounded `preview` are the only caller-supplied presentation text the index
//! permits; callers should keep the preview bounded before indexing.
//!
//! A source replacement is transactional. Callers only invoke it after a
//! complete provider scan succeeds, which means sessions omitted from the
//! replacement can safely be tombstoned without treating a transient source
//! failure as deletion. When more than one process can refresh the index,
//! callers also attach the cursor observed before scanning as an optimistic
//! precondition, so a stale scan cannot overwrite a newer replacement.

use std::{
  collections::HashSet,
  error::Error,
  fmt,
  path::Path,
  sync::{Mutex, MutexGuard},
  time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const APPLICATION_ID: i32 = 0x544f_4b4e; // "TOKN"

const MIGRATION_001: &str = r#"
CREATE TABLE sources (
  id INTEGER PRIMARY KEY,
  provider TEXT NOT NULL,
  source_key TEXT NOT NULL,
  cursor TEXT NOT NULL,
  scanned_at_ms INTEGER NOT NULL,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  UNIQUE(provider, source_key)
);

CREATE TABLE sessions (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE RESTRICT,
  session_id TEXT NOT NULL,
  source_path TEXT NOT NULL,
  title TEXT,
  preview TEXT,
  cwd TEXT,
  timestamp TEXT,
  updated_at TEXT,
  updated_at_ms INTEGER,
  parent_session_id TEXT,
  agent_path TEXT,
  agent_nickname TEXT,
  agent_role TEXT,
  attention_marker TEXT,
  attention_revision INTEGER NOT NULL DEFAULT 0 CHECK(attention_revision >= 0),
  seen_attention_revision INTEGER NOT NULL DEFAULT 0
    CHECK(seen_attention_revision >= 0 AND seen_attention_revision <= attention_revision),
  seen_at_ms INTEGER,
  present INTEGER NOT NULL DEFAULT 1 CHECK(present IN (0, 1)),
  first_indexed_at_ms INTEGER NOT NULL,
  indexed_at_ms INTEGER NOT NULL,
  UNIQUE(source_id, session_id)
);

CREATE INDEX sessions_by_source_and_updated
  ON sessions(source_id, present, updated_at_ms DESC, session_id);
CREATE INDEX sessions_by_source_and_parent
  ON sessions(source_id, parent_session_id, present, session_id);
CREATE INDEX sessions_by_attention
  ON sessions(present, attention_revision, seen_attention_revision);
"#;

// These fields distinguish a known-empty attention marker from a marker that
// has not been established yet. Existing indexes were written only after a
// complete body scan, so they migrate to the completed, non-notifying state.
const MIGRATION_002: &str = r#"
ALTER TABLE sessions
  ADD COLUMN attention_baselined INTEGER NOT NULL DEFAULT 1
    CHECK(attention_baselined IN (0, 1));

ALTER TABLE sessions
  ADD COLUMN notify_on_baseline INTEGER NOT NULL DEFAULT 0
    CHECK(notify_on_baseline IN (0, 1));
"#;

struct Migration {
  version: i64,
  sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
  Migration {
    version: 1,
    sql: MIGRATION_001,
  },
  Migration {
    version: 2,
    sql: MIGRATION_002,
  },
];

/// Errors returned by [`SessionIndex`].
#[derive(Debug)]
pub enum SessionIndexError {
  /// SQLite rejected an operation.
  Sqlite(rusqlite::Error),
  /// The index's serialized connection was poisoned by a previous panic.
  ConnectionPoisoned,
  /// A replacement does not describe one complete source snapshot.
  InvalidReplacement(String),
  /// A previously applied migration no longer matches the embedded migration.
  MigrationChecksumMismatch {
    version: i64,
    expected: String,
    found: String,
  },
  /// The database migration ledger is not a valid prefix of this crate's
  /// migrations, so continuing could misinterpret cached data.
  MigrationHistoryInvalid(String),
  /// The target is an existing SQLite database owned by another application.
  UnexpectedDatabase(i32),
  /// A session's monotonically increasing attention revision reached `i64::MAX`.
  AttentionRevisionOverflow,
  /// Another index writer changed a source after this replacement was planned.
  SourceCursorConflict {
    source: SourceKey,
    expected: SourceCursorPrecondition,
    actual: Option<String>,
  },
}

impl fmt::Display for SessionIndexError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Sqlite(error) => write!(formatter, "SQLite index error: {error}"),
      Self::ConnectionPoisoned => formatter.write_str("SQLite index connection lock was poisoned"),
      Self::InvalidReplacement(message) => write!(formatter, "invalid source replacement: {message}"),
      Self::MigrationChecksumMismatch {
        version,
        expected,
        found,
      } => write!(
        formatter,
        "migration {version} checksum mismatch (expected {expected}, found {found})"
      ),
      Self::MigrationHistoryInvalid(message) => write!(formatter, "invalid migration history: {message}"),
      Self::UnexpectedDatabase(application_id) => write!(
        formatter,
        "database belongs to another application (application_id {application_id})"
      ),
      Self::AttentionRevisionOverflow => formatter.write_str("attention revision overflow"),
      Self::SourceCursorConflict { source, .. } => write!(
        formatter,
        "source cursor changed before replacement for {}/{} could commit",
        source.provider, source.source_key
      ),
    }
  }
}

impl Error for SessionIndexError {
  fn source(&self) -> Option<&(dyn Error + 'static)> {
    match self {
      Self::Sqlite(error) => Some(error),
      _ => None,
    }
  }
}

impl From<rusqlite::Error> for SessionIndexError {
  fn from(error: rusqlite::Error) -> Self {
    Self::Sqlite(error)
  }
}

/// Result type returned by the index store.
pub type Result<T> = std::result::Result<T, SessionIndexError>;

/// Opaque provider-specific identity of one scanned source.
///
/// `source_key` is not required to be a path. For example, an app can use a
/// stable key for an OpenCode database while individual indexed sessions retain
/// their actual `source_path` for reopening.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceKey {
  pub provider: String,
  pub source_key: String,
}

impl SourceKey {
  pub fn new(provider: impl Into<String>, source_key: impl Into<String>) -> Self {
    Self {
      provider: provider.into(),
      source_key: source_key.into(),
    }
  }
}

/// Opaque source-aware identity of an indexed session.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SessionKey {
  pub provider: String,
  pub source_key: String,
  pub session_id: String,
}

impl SessionKey {
  pub fn new(provider: impl Into<String>, source_key: impl Into<String>, session_id: impl Into<String>) -> Self {
    Self {
      provider: provider.into(),
      source_key: source_key.into(),
      session_id: session_id.into(),
    }
  }

  pub fn source_key(&self) -> SourceKey {
    SourceKey {
      provider: self.provider.clone(),
      source_key: self.source_key.clone(),
    }
  }
}

/// Last successful scan state for one provider source.
///
/// `cursor` is intentionally opaque. It can represent a provider-specific
/// source revision, database/WAL fingerprint, or other safe checkpoint; this
/// crate never interprets it as a universal byte offset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceState {
  pub key: SourceKey,
  pub cursor: String,
  pub scanned_at_ms: i64,
}

impl SourceState {
  pub fn new(key: SourceKey, cursor: impl Into<String>, scanned_at_ms: i64) -> Self {
    Self {
      key,
      cursor: cursor.into(),
      scanned_at_ms,
    }
  }
}

/// A condition that must match the source state immediately before a
/// replacement commits.
///
/// Use [`SourceCursorPrecondition::Exact`] with `Some(cursor)` after reading a
/// source's prior [`SourceState`] to prevent a stale scan from overwriting a
/// newer snapshot committed by another process. `Exact(None)` requires the
/// source not to have been indexed yet. [`SourceCursorPrecondition::Any`] is
/// the compatibility default for callers that do not need optimistic
/// concurrency control.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SourceCursorPrecondition {
  /// Do not compare the current source state before replacing it.
  #[default]
  Any,
  /// Require the current cursor to exactly match this value. `None` requires
  /// that the source does not exist in the index.
  Exact(Option<String>),
}

impl SourceCursorPrecondition {
  /// Requires an already indexed source to have this exact cursor.
  pub fn existing(cursor: impl Into<String>) -> Self {
    Self::Exact(Some(cursor.into()))
  }

  /// Requires the source not to have been indexed yet.
  pub const fn missing() -> Self {
    Self::Exact(None)
  }

  fn is_satisfied_by(&self, actual_cursor: Option<&str>) -> bool {
    match self {
      Self::Any => true,
      Self::Exact(expected_cursor) => expected_cursor.as_deref() == actual_cursor,
    }
  }
}

/// Session metadata persisted by the index.
///
/// The title and preview are copied exactly as supplied by the caller. The
/// store does not sanitize, derive, or inspect either field. `attention_marker`
/// must be an opaque token (for example, a message ID or fingerprint), never a
/// message body. `attention_baselined` records whether the caller has inspected
/// the body enough to establish that marker; a missing marker is meaningful only
/// when it is true. `notify_on_baseline` lets a caller defer the unread decision
/// until that first body inspection. `has_new_attention` is the separate,
/// caller-controlled signal that one or more new visible user messages or final
/// assistant messages have been observed. Metadata changes, marker rewrites,
/// and history reductions must leave it false.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMetadata {
  pub key: SessionKey,
  /// The actual source path or other reopenable source locator serialized as a
  /// string. It is distinct from the source's opaque `source_key`.
  pub source_path: String,
  pub title: Option<String>,
  pub preview: Option<String>,
  pub cwd: Option<String>,
  /// Provider-native creation time, retained as supplied.
  pub timestamp: Option<String>,
  /// Provider-native last-update representation, retained as supplied.
  pub updated_at: Option<String>,
  /// Canonical Unix milliseconds used for local ordering when available.
  pub updated_at_ms: Option<i64>,
  pub parent_session_id: Option<String>,
  pub agent_path: Option<String>,
  pub agent_nickname: Option<String>,
  pub agent_role: Option<String>,
  pub attention_marker: Option<String>,
  /// Whether `attention_marker` reflects a completed body inspection.
  pub attention_baselined: bool,
  /// Whether the first completed body inspection should make eligible visible
  /// conversation activity unread. This is only valid while not baselined.
  pub notify_on_baseline: bool,
  /// Advances the unread revision only when the caller observed new eligible
  /// visible conversation activity. The index cannot infer this from metadata.
  pub has_new_attention: bool,
}

impl SessionMetadata {
  pub fn new(key: SessionKey, source_path: impl Into<String>) -> Self {
    Self {
      key,
      source_path: source_path.into(),
      title: None,
      preview: None,
      cwd: None,
      timestamp: None,
      updated_at: None,
      updated_at_ms: None,
      parent_session_id: None,
      agent_path: None,
      agent_nickname: None,
      agent_role: None,
      attention_marker: None,
      attention_baselined: true,
      notify_on_baseline: false,
      has_new_attention: false,
    }
  }
}

/// Whether a source replacement establishes a notification baseline or tracks
/// new visible conversation activity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AttentionMode {
  /// Store incoming markers but do not leave them unread. Use for an explicit
  /// initial baseline or a deliberate acknowledgement reset.
  Baseline,
  /// Increment attention revisions only when the caller reports new eligible
  /// visible conversation activity.
  #[default]
  TrackChanges,
}

/// A complete, successfully scanned source inventory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceReplacement {
  pub source: SourceState,
  pub sessions: Vec<SessionMetadata>,
  pub attention_mode: AttentionMode,
  /// Optional optimistic guard for the source state read before scanning.
  /// Constructors default this to [`SourceCursorPrecondition::Any`] for
  /// backward-compatible convenience. New cross-process callers should set
  /// it to an exact state with [`Self::with_source_cursor_precondition`].
  pub source_cursor_precondition: SourceCursorPrecondition,
}

impl SourceReplacement {
  pub fn new(source: SourceState, sessions: Vec<SessionMetadata>) -> Self {
    Self {
      source,
      sessions,
      attention_mode: AttentionMode::TrackChanges,
      source_cursor_precondition: SourceCursorPrecondition::Any,
    }
  }

  pub fn baseline(source: SourceState, sessions: Vec<SessionMetadata>) -> Self {
    Self {
      source,
      sessions,
      attention_mode: AttentionMode::Baseline,
      source_cursor_precondition: SourceCursorPrecondition::Any,
    }
  }

  /// Adds an optimistic source-state guard to this replacement.
  ///
  /// Capture the expected cursor before scanning, then use
  /// [`SourceCursorPrecondition::existing`] for a known source or
  /// [`SourceCursorPrecondition::missing`] for a source that was absent. If
  /// another writer changes that source before this replacement commits,
  /// [`SessionIndex::replace_source`] or [`SessionIndex::replace_sources`] returns
  /// [`SessionIndexError::SourceCursorConflict`] without changing the index.
  pub fn with_source_cursor_precondition(mut self, source_cursor_precondition: SourceCursorPrecondition) -> Self {
    self.source_cursor_precondition = source_cursor_precondition;
    self
  }
}

/// Metadata read back from the index, including derived notification state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexedSession {
  pub key: SessionKey,
  pub source_path: String,
  pub title: Option<String>,
  pub preview: Option<String>,
  pub cwd: Option<String>,
  pub timestamp: Option<String>,
  pub updated_at: Option<String>,
  pub updated_at_ms: Option<i64>,
  pub parent_session_id: Option<String>,
  pub agent_path: Option<String>,
  pub agent_nickname: Option<String>,
  pub agent_role: Option<String>,
  pub attention_marker: Option<String>,
  /// Whether `attention_marker` reflects a completed body inspection.
  pub attention_baselined: bool,
  /// Whether a first completed body inspection should surface eligible visible
  /// conversation activity as unread.
  pub notify_on_baseline: bool,
  pub attention_revision: i64,
  pub seen_attention_revision: i64,
  pub seen_at_ms: Option<i64>,
  pub present: bool,
}

impl IndexedSession {
  pub fn has_unread(&self) -> bool {
    self.attention_revision > self.seen_attention_revision
  }
}

/// Result details from one committed source replacement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplaceSummary {
  pub inserted: usize,
  pub updated: usize,
  pub tombstoned: usize,
  pub attention_changed: usize,
  pub baseline_established: bool,
}

/// Outcome of completing one cataloged session's attention baseline.
///
/// [`Self::Stale`] is expected when a later catalog replacement changed the
/// source cursor, removed the session, or another worker already completed the
/// pending baseline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionBaselineCompletion {
  Applied {
    /// Whether this completion advanced the unread attention revision.
    attention_changed: bool,
  },
  Stale,
}

impl SessionBaselineCompletion {
  pub const fn was_applied(self) -> bool {
    matches!(self, Self::Applied { .. })
  }

  pub const fn attention_changed(self) -> bool {
    matches!(
      self,
      Self::Applied {
        attention_changed: true
      }
    )
  }
}

/// Body-derived attention data used to complete one cataloged session baseline.
///
/// This intentionally excludes catalog-owned presentation and relationship
/// metadata. A delayed body load must not overwrite a title, source path, or
/// parent relationship that a same-cursor catalog pass has already refreshed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionBaselineCompletionRequest {
  pub key: SessionKey,
  /// Opaque body-derived marker, such as the newest eligible message ID.
  pub attention_marker: Option<String>,
  /// Whether the completed baseline should advance the unread revision.
  pub has_new_attention: bool,
}

impl SessionBaselineCompletionRequest {
  pub fn new(key: SessionKey, attention_marker: Option<String>) -> Self {
    Self {
      key,
      attention_marker,
      has_new_attention: false,
    }
  }
}

/// An app-owned SQLite session index.
///
/// All access takes a single connection lock. That deliberately serializes
/// writers and readers in-process, while SQLite WAL and the busy timeout make
/// a concurrent process fail predictably rather than corrupting cached state.
pub struct SessionIndex {
  connection: Mutex<Connection>,
}

impl fmt::Debug for SessionIndex {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.debug_struct("SessionIndex").finish_non_exhaustive()
  }
}

impl SessionIndex {
  /// Opens (and migrates) an app-owned index database.
  pub fn open(path: impl AsRef<Path>) -> Result<Self> {
    Self::from_connection(Connection::open(path)?)
  }

  /// Opens an in-memory index. This is useful for tests and isolated previews.
  pub fn open_in_memory() -> Result<Self> {
    Self::from_connection(Connection::open_in_memory()?)
  }

  fn from_connection(mut connection: Connection) -> Result<Self> {
    ensure_ownership(&mut connection)?;
    configure_connection(&mut connection)?;
    migrate(&mut connection)?;
    Ok(Self {
      connection: Mutex::new(connection),
    })
  }

  /// Returns the last successful state for one source.
  pub fn source_state(&self, key: &SourceKey) -> Result<Option<SourceState>> {
    let connection = self.connection()?;
    connection
      .query_row(
        "SELECT cursor, scanned_at_ms FROM sources WHERE provider = ?1 AND source_key = ?2",
        params![key.provider, key.source_key],
        |row| {
          Ok(SourceState {
            key: key.clone(),
            cursor: row.get(0)?,
            scanned_at_ms: row.get(1)?,
          })
        },
      )
      .optional()
      .map_err(Into::into)
  }

  /// Lists every successfully indexed source for a provider.
  pub fn list_sources(&self, provider: &str) -> Result<Vec<SourceState>> {
    let connection = self.connection()?;
    let mut statement = connection.prepare(
      "SELECT source_key, cursor, scanned_at_ms
       FROM sources
       WHERE provider = ?1
       ORDER BY source_key ASC",
    )?;
    let rows = statement.query_map(params![provider], |row| {
      Ok(SourceState {
        key: SourceKey {
          provider: provider.to_owned(),
          source_key: row.get(0)?,
        },
        cursor: row.get(1)?,
        scanned_at_ms: row.get(2)?,
      })
    })?;
    collect_rows(rows)
  }

  /// Reports whether this provider has committed at least one successful source
  /// replacement. Apps that need whole-provider readiness should persist their
  /// own empty catalog sentinel only after every source baseline succeeds.
  pub fn has_sources_for_provider(&self, provider: &str) -> Result<bool> {
    let connection = self.connection()?;
    let exists: i64 = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sources WHERE provider = ?1)",
      params![provider],
      |row| row.get(0),
    )?;
    Ok(exists != 0)
  }

  /// Transactionally replaces the complete inventory for one successfully
  /// scanned source.
  ///
  /// Sessions not included in `replacement.sessions` are tombstoned only after
  /// the whole transaction commits. A failed scan should never call this method.
  /// This is the single-source compatibility wrapper for
  /// [`Self::replace_sources`].
  pub fn replace_source(&self, replacement: SourceReplacement) -> Result<ReplaceSummary> {
    let mut summaries = self.replace_sources(std::slice::from_ref(&replacement))?;
    Ok(
      summaries
        .pop()
        .expect("a single source replacement must produce one summary"),
    )
  }

  /// Transactionally replaces the complete inventories for every successfully
  /// scanned source in `replacements`.
  ///
  /// The batch rejects duplicate source keys because each replacement must be
  /// one source's complete snapshot. It validates every replacement and every
  /// optimistic cursor precondition before writing any source, then commits
  /// every source update together. Therefore, a stale source anywhere in the
  /// batch leaves every source unchanged.
  ///
  /// Sessions omitted from one replacement are tombstoned only after the whole
  /// batch commits. A failed scan should never include that source in a batch.
  pub fn replace_sources(&self, replacements: &[SourceReplacement]) -> Result<Vec<ReplaceSummary>> {
    validate_replacements(replacements)?;

    let mut connection = self.connection()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_source_cursor_preconditions(&transaction, replacements)?;
    let summaries = replacements
      .iter()
      .map(|replacement| replace_source_in_transaction(&transaction, replacement))
      .collect::<Result<Vec<_>>>()?;
    transaction.commit()?;
    Ok(summaries)
  }

  /// Completes the pending attention baseline for exactly one cataloged
  /// session, without replacing or tombstoning sibling sessions.
  ///
  /// `expected_source` must be the staged source state captured by the catalog
  /// pass. `next_source` must have the same key and a new cursor. The completion
  /// atomically advances to that cursor so a catalog replacement still guarded
  /// by the staged cursor cannot regress the completed attention state. The
  /// completion applies only while the staged cursor still matches and the
  /// target session remains present with `attention_baselined == false`.
  /// Callers provide only body-derived attention data; catalog-owned metadata
  /// remains unchanged. A mismatched or already-completed row returns
  /// [`SessionBaselineCompletion::Stale`] rather than overwriting newer data.
  pub fn complete_session_baseline(
    &self,
    expected_source: &SourceState,
    next_source: &SourceState,
    request: SessionBaselineCompletionRequest,
  ) -> Result<SessionBaselineCompletion> {
    validate_session_baseline_completion(expected_source, next_source, &request)?;

    let mut connection = self.connection()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let source_id = transaction
      .query_row(
        "SELECT id FROM sources WHERE provider = ?1 AND source_key = ?2 AND cursor = ?3",
        params![
          expected_source.key.provider,
          expected_source.key.source_key,
          expected_source.cursor,
        ],
        |row| row.get::<_, i64>(0),
      )
      .optional()?;
    let Some(source_id) = source_id else {
      return Ok(SessionBaselineCompletion::Stale);
    };

    let pending = pending_session_for_completion(&transaction, source_id, &request.key.session_id)?;
    let Some(pending) = pending else {
      return Ok(SessionBaselineCompletion::Stale);
    };
    if !pending.present || pending.attention_baselined {
      return Ok(SessionBaselineCompletion::Stale);
    }

    let attention_changed = request.has_new_attention;
    let attention_revision = if attention_changed {
      next_attention_revision(pending.attention_revision)?
    } else {
      pending.attention_revision
    };
    complete_session_attention_baseline(
      &transaction,
      pending.id,
      &request,
      attention_revision,
      pending.seen_attention_revision,
    )?;
    advance_source_state(&transaction, source_id, next_source)?;
    transaction.commit()?;
    Ok(SessionBaselineCompletion::Applied { attention_changed })
  }

  /// Returns one session, including tombstoned rows when present in the index.
  pub fn session(&self, key: &SessionKey) -> Result<Option<IndexedSession>> {
    let connection = self.connection()?;
    select_session(
      &connection,
      "WHERE source.provider = ?1 AND source.source_key = ?2 AND session.session_id = ?3",
      params![key.provider, key.source_key, key.session_id],
    )
  }

  /// Lists all currently present sessions in stable update order.
  pub fn list_present_sessions(&self) -> Result<Vec<IndexedSession>> {
    let connection = self.connection()?;
    select_sessions(&connection, "WHERE session.present = 1", [])
  }

  /// Lists every indexed session, including tombstoned rows.
  pub fn list_all_sessions(&self) -> Result<Vec<IndexedSession>> {
    let connection = self.connection()?;
    select_sessions(&connection, "", [])
  }

  /// Acknowledges attention only through the revision captured by a successful
  /// event-page response. Call this only after the app has accepted that page.
  ///
  /// The acknowledgement is monotonic and clamps to the current revision, so a
  /// stale or cancelled page request cannot erase newer attention indexed while
  /// it was loading. Returns whether the seen revision actually advanced.
  pub fn mark_seen_through(&self, key: &SessionKey, attention_revision: i64, seen_at_ms: i64) -> Result<bool> {
    if attention_revision < 0 {
      return Err(SessionIndexError::InvalidReplacement(
        "seen attention revision cannot be negative".to_owned(),
      ));
    }
    let connection = self.connection()?;
    let updated = connection.execute(
      "UPDATE sessions
       SET seen_attention_revision = MIN(attention_revision, ?4),
           seen_at_ms = ?5
       WHERE id = (
         SELECT session.id
         FROM sessions AS session
         INNER JOIN sources AS source ON source.id = session.source_id
         WHERE source.provider = ?1
           AND source.source_key = ?2
           AND session.session_id = ?3
       )
       AND seen_attention_revision < MIN(attention_revision, ?4)",
      params![
        key.provider,
        key.source_key,
        key.session_id,
        attention_revision,
        seen_at_ms
      ],
    )?;
    Ok(updated != 0)
  }

  fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
    self
      .connection
      .lock()
      .map_err(|_| SessionIndexError::ConnectionPoisoned)
  }
}

fn configure_connection(connection: &mut Connection) -> Result<()> {
  connection.busy_timeout(BUSY_TIMEOUT)?;
  connection.pragma_update(None, "foreign_keys", "ON")?;
  connection.pragma_update(None, "journal_mode", "WAL")?;
  Ok(())
}

fn ensure_ownership(connection: &mut Connection) -> Result<()> {
  let application_id: i32 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
  if application_id == APPLICATION_ID {
    return Ok(());
  }
  if application_id != 0 {
    return Err(SessionIndexError::UnexpectedDatabase(application_id));
  }

  let existing_object_count: i64 = connection.query_row(
    "SELECT COUNT(*)
     FROM sqlite_schema
     WHERE name NOT LIKE 'sqlite_%'",
    [],
    |row| row.get(0),
  )?;
  if existing_object_count != 0 {
    return Err(SessionIndexError::UnexpectedDatabase(application_id));
  }
  connection.pragma_update(None, "application_id", APPLICATION_ID)?;
  Ok(())
}

fn migrate(connection: &mut Connection) -> Result<()> {
  connection.execute_batch(
    "CREATE TABLE IF NOT EXISTS schema_migrations (
       version INTEGER PRIMARY KEY,
       checksum TEXT NOT NULL,
       applied_at_ms INTEGER NOT NULL
     );",
  )?;

  let applied = {
    let mut statement = connection.prepare("SELECT version, checksum FROM schema_migrations ORDER BY version ASC")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?;
    collect_rows(rows)?
  };

  if applied.len() > MIGRATIONS.len() {
    return Err(SessionIndexError::MigrationHistoryInvalid(format!(
      "database has {} migrations but this build knows {}",
      applied.len(),
      MIGRATIONS.len()
    )));
  }

  for (index, (version, found_checksum)) in applied.iter().enumerate() {
    let migration = MIGRATIONS
      .get(index)
      .ok_or_else(|| SessionIndexError::MigrationHistoryInvalid(format!("unexpected migration {version}")))?;
    if *version != migration.version {
      return Err(SessionIndexError::MigrationHistoryInvalid(format!(
        "expected migration {} at position {}, found {version}",
        migration.version,
        index + 1
      )));
    }

    let expected_checksum = migration_checksum(migration.sql);
    if *found_checksum != expected_checksum {
      return Err(SessionIndexError::MigrationChecksumMismatch {
        version: *version,
        expected: expected_checksum,
        found: found_checksum.clone(),
      });
    }
  }

  for migration in MIGRATIONS.iter().skip(applied.len()) {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(migration.sql)?;
    transaction.execute(
      "INSERT INTO schema_migrations(version, checksum, applied_at_ms) VALUES(?1, ?2, ?3)",
      params![migration.version, migration_checksum(migration.sql), current_time_ms()],
    )?;
    transaction.commit()?;
  }

  Ok(())
}

fn migration_checksum(sql: &str) -> String {
  // FNV-1a is sufficient here as an immutable-migration drift guard and avoids
  // bringing a cryptographic dependency into every app that uses the index.
  let mut hash = 0xcbf2_9ce4_8422_2325_u64;
  for byte in sql.bytes() {
    hash ^= u64::from(byte);
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
  }
  format!("{hash:016x}")
}

fn current_time_ms() -> i64 {
  let milliseconds = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis();
  i64::try_from(milliseconds).unwrap_or(i64::MAX)
}

fn validate_replacement(replacement: &SourceReplacement) -> Result<()> {
  let mut session_ids = HashSet::with_capacity(replacement.sessions.len());
  for session in &replacement.sessions {
    if session.key.provider != replacement.source.key.provider
      || session.key.source_key != replacement.source.key.source_key
    {
      return Err(SessionIndexError::InvalidReplacement(format!(
        "session {} does not belong to source {}/{}",
        session.key.session_id, replacement.source.key.provider, replacement.source.key.source_key
      )));
    }
    if session.attention_baselined && session.notify_on_baseline {
      return Err(SessionIndexError::InvalidReplacement(format!(
        "session {} cannot notify on an already established attention baseline",
        session.key.session_id
      )));
    }
    if !session_ids.insert(session.key.session_id.as_str()) {
      return Err(SessionIndexError::InvalidReplacement(format!(
        "source contains duplicate session id {}",
        session.key.session_id
      )));
    }
  }
  Ok(())
}

fn validate_replacements(replacements: &[SourceReplacement]) -> Result<()> {
  let mut source_keys = HashSet::with_capacity(replacements.len());
  for replacement in replacements {
    validate_replacement(replacement)?;
    if !source_keys.insert(&replacement.source.key) {
      return Err(SessionIndexError::InvalidReplacement(format!(
        "batch contains duplicate source {}/{}",
        replacement.source.key.provider, replacement.source.key.source_key
      )));
    }
  }
  Ok(())
}

fn validate_session_baseline_completion(
  expected_source: &SourceState,
  next_source: &SourceState,
  request: &SessionBaselineCompletionRequest,
) -> Result<()> {
  if request.key.provider != expected_source.key.provider || request.key.source_key != expected_source.key.source_key {
    return Err(SessionIndexError::InvalidReplacement(format!(
      "session {} does not belong to staged source {}/{}",
      request.key.session_id, expected_source.key.provider, expected_source.key.source_key
    )));
  }
  if next_source.key != expected_source.key {
    return Err(SessionIndexError::InvalidReplacement(format!(
      "next source {}/{} does not match staged source {}/{}",
      next_source.key.provider,
      next_source.key.source_key,
      expected_source.key.provider,
      expected_source.key.source_key
    )));
  }
  if next_source.cursor == expected_source.cursor {
    return Err(SessionIndexError::InvalidReplacement(format!(
      "next source cursor for {}/{} must advance beyond the staged cursor",
      expected_source.key.provider, expected_source.key.source_key
    )));
  }
  Ok(())
}

fn validate_source_cursor_preconditions(
  transaction: &Transaction<'_>,
  replacements: &[SourceReplacement],
) -> Result<()> {
  for replacement in replacements {
    let actual = transaction
      .query_row(
        "SELECT cursor FROM sources WHERE provider = ?1 AND source_key = ?2",
        params![replacement.source.key.provider, replacement.source.key.source_key],
        |row| row.get::<_, String>(0),
      )
      .optional()?;
    if !replacement
      .source_cursor_precondition
      .is_satisfied_by(actual.as_deref())
    {
      return Err(SessionIndexError::SourceCursorConflict {
        source: replacement.source.key.clone(),
        expected: replacement.source_cursor_precondition.clone(),
        actual,
      });
    }
  }
  Ok(())
}

fn replace_source_in_transaction(
  transaction: &Transaction<'_>,
  replacement: &SourceReplacement,
) -> Result<ReplaceSummary> {
  let existing_source = transaction
    .query_row(
      "SELECT id FROM sources WHERE provider = ?1 AND source_key = ?2",
      params![replacement.source.key.provider, replacement.source.key.source_key],
      |row| Ok(ExistingSource { id: row.get(0)? }),
    )
    .optional()?;

  let baseline_established = replacement.attention_mode == AttentionMode::Baseline;
  let source_id = match existing_source {
    Some(source) => {
      transaction.execute(
        "UPDATE sources
         SET cursor = ?3, scanned_at_ms = ?4, updated_at_ms = ?4
         WHERE provider = ?1 AND source_key = ?2",
        params![
          replacement.source.key.provider,
          replacement.source.key.source_key,
          replacement.source.cursor,
          replacement.source.scanned_at_ms,
        ],
      )?;
      source.id
    }
    None => {
      transaction.execute(
        "INSERT INTO sources(provider, source_key, cursor, scanned_at_ms, created_at_ms, updated_at_ms)
         VALUES(?1, ?2, ?3, ?4, ?4, ?4)",
        params![
          replacement.source.key.provider,
          replacement.source.key.source_key,
          replacement.source.cursor,
          replacement.source.scanned_at_ms,
        ],
      )?;
      transaction.last_insert_rowid()
    }
  };

  let previously_present = {
    let mut statement = transaction.prepare("SELECT session_id FROM sessions WHERE source_id = ?1 AND present = 1")?;
    let rows = statement.query_map(params![source_id], |row| row.get::<_, String>(0))?;
    collect_rows(rows)?.into_iter().collect::<HashSet<_>>()
  };
  let incoming_session_ids = replacement
    .sessions
    .iter()
    .map(|session| session.key.session_id.as_str())
    .collect::<HashSet<_>>();
  let tombstoned = previously_present
    .iter()
    .filter(|session_id| !incoming_session_ids.contains(session_id.as_str()))
    .count();

  // Marking before upsert keeps the tombstone statement simple. The surrounding
  // transaction means readers observe either the old complete inventory or the
  // new complete inventory, never this temporary state.
  transaction.execute(
    "UPDATE sessions SET present = 0, indexed_at_ms = ?2 WHERE source_id = ?1 AND present = 1",
    params![source_id, replacement.source.scanned_at_ms],
  )?;

  let mut summary = ReplaceSummary {
    tombstoned,
    baseline_established,
    ..ReplaceSummary::default()
  };
  for session in &replacement.sessions {
    let existing = existing_session_attention(transaction, source_id, &session.key.session_id)?;
    match existing {
      Some(existing) => {
        summary.updated += 1;
        let attention_revision = if session.has_new_attention && !baseline_established {
          let revision = next_attention_revision(existing.attention_revision)?;
          summary.attention_changed += 1;
          revision
        } else {
          existing.attention_revision
        };
        let seen_attention_revision = if baseline_established {
          attention_revision
        } else {
          existing.seen_attention_revision
        };
        update_session(
          transaction,
          existing.id,
          session,
          attention_revision,
          seen_attention_revision,
          replacement.source.scanned_at_ms,
        )?;
      }
      None => {
        summary.inserted += 1;
        let attention_revision = if session.has_new_attention && !baseline_established {
          1
        } else {
          0
        };
        let seen_attention_revision = 0;
        if session.has_new_attention && !baseline_established {
          summary.attention_changed += 1;
        }
        insert_session(
          transaction,
          source_id,
          session,
          attention_revision,
          seen_attention_revision,
          replacement.source.scanned_at_ms,
        )?;
      }
    }
  }

  Ok(summary)
}

#[derive(Debug)]
struct ExistingSource {
  id: i64,
}

#[derive(Debug)]
struct ExistingSessionAttention {
  id: i64,
  attention_revision: i64,
  seen_attention_revision: i64,
}

#[derive(Debug)]
struct PendingSessionCompletion {
  id: i64,
  attention_revision: i64,
  seen_attention_revision: i64,
  present: bool,
  attention_baselined: bool,
}

fn existing_session_attention(
  transaction: &Transaction<'_>,
  source_id: i64,
  session_id: &str,
) -> Result<Option<ExistingSessionAttention>> {
  transaction
    .query_row(
      "SELECT id, attention_revision, seen_attention_revision
       FROM sessions
       WHERE source_id = ?1 AND session_id = ?2",
      params![source_id, session_id],
      |row| {
        Ok(ExistingSessionAttention {
          id: row.get(0)?,
          attention_revision: row.get(1)?,
          seen_attention_revision: row.get(2)?,
        })
      },
    )
    .optional()
    .map_err(Into::into)
}

fn pending_session_for_completion(
  transaction: &Transaction<'_>,
  source_id: i64,
  session_id: &str,
) -> Result<Option<PendingSessionCompletion>> {
  transaction
    .query_row(
      "SELECT id, attention_revision, seen_attention_revision, present, attention_baselined
       FROM sessions
       WHERE source_id = ?1 AND session_id = ?2",
      params![source_id, session_id],
      |row| {
        Ok(PendingSessionCompletion {
          id: row.get(0)?,
          attention_revision: row.get(1)?,
          seen_attention_revision: row.get(2)?,
          present: row.get(3)?,
          attention_baselined: row.get(4)?,
        })
      },
    )
    .optional()
    .map_err(Into::into)
}

fn complete_session_attention_baseline(
  transaction: &Transaction<'_>,
  id: i64,
  request: &SessionBaselineCompletionRequest,
  attention_revision: i64,
  seen_attention_revision: i64,
) -> Result<()> {
  transaction.execute(
    "UPDATE sessions
     SET attention_marker = ?2,
         attention_baselined = 1,
         notify_on_baseline = 0,
         attention_revision = ?3,
         seen_attention_revision = ?4
     WHERE id = ?1",
    params![
      id,
      request.attention_marker,
      attention_revision,
      seen_attention_revision,
    ],
  )?;
  Ok(())
}

fn advance_source_state(transaction: &Transaction<'_>, source_id: i64, next_source: &SourceState) -> Result<()> {
  transaction.execute(
    "UPDATE sources
     SET cursor = ?2,
         scanned_at_ms = ?3,
         updated_at_ms = ?3
     WHERE id = ?1",
    params![source_id, next_source.cursor, next_source.scanned_at_ms],
  )?;
  Ok(())
}

fn next_attention_revision(revision: i64) -> Result<i64> {
  revision
    .checked_add(1)
    .ok_or(SessionIndexError::AttentionRevisionOverflow)
}

fn insert_session(
  transaction: &Transaction<'_>,
  source_id: i64,
  session: &SessionMetadata,
  attention_revision: i64,
  seen_attention_revision: i64,
  indexed_at_ms: i64,
) -> Result<()> {
  transaction.execute(
    "INSERT INTO sessions(
       source_id, session_id, source_path, title, preview, cwd, timestamp,
       updated_at, updated_at_ms, parent_session_id, agent_path, agent_nickname,
       agent_role, attention_marker, attention_baselined, notify_on_baseline,
       attention_revision, seen_attention_revision, seen_at_ms, present,
       first_indexed_at_ms, indexed_at_ms
     ) VALUES(
       ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
       ?17, ?18, NULL, 1, ?19, ?19
     )",
    params![
      source_id,
      session.key.session_id,
      session.source_path,
      session.title,
      session.preview,
      session.cwd,
      session.timestamp,
      session.updated_at,
      session.updated_at_ms,
      session.parent_session_id,
      session.agent_path,
      session.agent_nickname,
      session.agent_role,
      session.attention_marker,
      session.attention_baselined,
      session.notify_on_baseline,
      attention_revision,
      seen_attention_revision,
      indexed_at_ms,
    ],
  )?;
  Ok(())
}

fn update_session(
  transaction: &Transaction<'_>,
  id: i64,
  session: &SessionMetadata,
  attention_revision: i64,
  seen_attention_revision: i64,
  indexed_at_ms: i64,
) -> Result<()> {
  transaction.execute(
    "UPDATE sessions
     SET source_path = ?2,
         title = ?3,
         preview = ?4,
         cwd = ?5,
         timestamp = ?6,
         updated_at = ?7,
         updated_at_ms = ?8,
         parent_session_id = ?9,
         agent_path = ?10,
         agent_nickname = ?11,
         agent_role = ?12,
         attention_marker = ?13,
         attention_baselined = ?14,
         notify_on_baseline = ?15,
         attention_revision = ?16,
         seen_attention_revision = ?17,
         present = 1,
         indexed_at_ms = ?18
     WHERE id = ?1",
    params![
      id,
      session.source_path,
      session.title,
      session.preview,
      session.cwd,
      session.timestamp,
      session.updated_at,
      session.updated_at_ms,
      session.parent_session_id,
      session.agent_path,
      session.agent_nickname,
      session.agent_role,
      session.attention_marker,
      session.attention_baselined,
      session.notify_on_baseline,
      attention_revision,
      seen_attention_revision,
      indexed_at_ms,
    ],
  )?;
  Ok(())
}

const SESSION_COLUMNS: &str = "
  source.provider,
  source.source_key,
  session.session_id,
  session.source_path,
  session.title,
  session.preview,
  session.cwd,
  session.timestamp,
  session.updated_at,
  session.updated_at_ms,
  session.parent_session_id,
  session.agent_path,
  session.agent_nickname,
  session.agent_role,
  session.attention_marker,
  session.attention_baselined,
  session.notify_on_baseline,
  session.attention_revision,
  session.seen_attention_revision,
  session.seen_at_ms,
  session.present";

fn select_session<P>(connection: &Connection, where_clause: &str, parameters: P) -> Result<Option<IndexedSession>>
where
  P: rusqlite::Params,
{
  let sql = format!(
    "SELECT {SESSION_COLUMNS}
     FROM sessions AS session
     INNER JOIN sources AS source ON source.id = session.source_id
     {where_clause}"
  );
  connection
    .query_row(&sql, parameters, indexed_session_from_row)
    .optional()
    .map_err(Into::into)
}

fn select_sessions<P>(connection: &Connection, where_clause: &str, parameters: P) -> Result<Vec<IndexedSession>>
where
  P: rusqlite::Params,
{
  let sql = format!(
    "SELECT {SESSION_COLUMNS}
     FROM sessions AS session
     INNER JOIN sources AS source ON source.id = session.source_id
     {where_clause}
     ORDER BY
       CASE WHEN session.updated_at_ms IS NULL THEN 1 ELSE 0 END ASC,
       session.updated_at_ms DESC,
       session.session_id ASC,
       source.provider ASC,
       source.source_key ASC"
  );
  let mut statement = connection.prepare(&sql)?;
  let rows = statement.query_map(parameters, indexed_session_from_row)?;
  collect_rows(rows)
}

fn indexed_session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexedSession> {
  let present: i64 = row.get(20)?;
  Ok(IndexedSession {
    key: SessionKey {
      provider: row.get(0)?,
      source_key: row.get(1)?,
      session_id: row.get(2)?,
    },
    source_path: row.get(3)?,
    title: row.get(4)?,
    preview: row.get(5)?,
    cwd: row.get(6)?,
    timestamp: row.get(7)?,
    updated_at: row.get(8)?,
    updated_at_ms: row.get(9)?,
    parent_session_id: row.get(10)?,
    agent_path: row.get(11)?,
    agent_nickname: row.get(12)?,
    agent_role: row.get(13)?,
    attention_marker: row.get(14)?,
    attention_baselined: row.get(15)?,
    notify_on_baseline: row.get(16)?,
    attention_revision: row.get(17)?,
    seen_attention_revision: row.get(18)?,
    seen_at_ms: row.get(19)?,
    present: present != 0,
  })
}

fn collect_rows<T>(
  rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
  rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
  use rusqlite::Connection;
  use tempfile::tempdir;

  use super::*;

  const PROVIDER: &str = "codex";
  const SOURCE_KEY: &str = "source-a";

  fn source(cursor: &str, scanned_at_ms: i64) -> SourceState {
    source_for(PROVIDER, SOURCE_KEY, cursor, scanned_at_ms)
  }

  fn source_for(provider: &str, source_key: &str, cursor: &str, scanned_at_ms: i64) -> SourceState {
    SourceState::new(SourceKey::new(provider, source_key), cursor, scanned_at_ms)
  }

  fn session(id: &str, marker: Option<&str>) -> SessionMetadata {
    session_for(PROVIDER, SOURCE_KEY, id, marker)
  }

  fn session_for(provider: &str, source_key: &str, id: &str, marker: Option<&str>) -> SessionMetadata {
    let mut session = SessionMetadata::new(
      SessionKey::new(provider, source_key, id),
      format!("/sessions/{id}.jsonl"),
    );
    session.title = Some(format!("Title {id}"));
    session.preview = Some(format!("Preview {id}"));
    session.timestamp = Some("2026-09-01T00:00:00Z".to_owned());
    session.updated_at = Some("provider-update".to_owned());
    session.updated_at_ms = Some(1_000);
    session.attention_marker = marker.map(str::to_owned);
    session
  }

  fn replacement(cursor: &str, scanned_at_ms: i64, sessions: Vec<SessionMetadata>) -> SourceReplacement {
    SourceReplacement::new(source(cursor, scanned_at_ms), sessions)
  }

  fn baseline_replacement(cursor: &str, scanned_at_ms: i64, sessions: Vec<SessionMetadata>) -> SourceReplacement {
    SourceReplacement::baseline(source(cursor, scanned_at_ms), sessions)
  }

  fn pending_session(id: &str, notify_on_baseline: bool) -> SessionMetadata {
    let mut session = session(id, None);
    session.attention_baselined = false;
    session.notify_on_baseline = notify_on_baseline;
    session
  }

  fn completion_request(id: &str, marker: Option<&str>, has_new_attention: bool) -> SessionBaselineCompletionRequest {
    let mut request =
      SessionBaselineCompletionRequest::new(SessionKey::new(PROVIDER, SOURCE_KEY, id), marker.map(str::to_owned));
    request.has_new_attention = has_new_attention;
    request
  }

  #[test]
  fn creates_and_records_migrations() {
    let directory = tempdir().expect("temporary directory should exist");
    let path = directory.path().join("session-index.sqlite3");
    let index = SessionIndex::open(&path).expect("index should open");
    drop(index);

    let connection = Connection::open(&path).expect("database should reopen");
    let applied: Vec<(i64, String)> = connection
      .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")
      .expect("migration query should prepare")
      .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
      .expect("migration query should run")
      .collect::<rusqlite::Result<_>>()
      .expect("migration rows should decode");
    assert_eq!(
      applied,
      vec![
        (1, migration_checksum(MIGRATION_001)),
        (2, migration_checksum(MIGRATION_002)),
      ]
    );
  }

  #[test]
  fn migration_preserves_existing_rows_as_completed_non_notifying_baselines() {
    let directory = tempdir().expect("temporary directory should exist");
    let path = directory.path().join("session-index.sqlite3");
    let connection = Connection::open(&path).expect("database should open");
    connection
      .pragma_update(None, "application_id", APPLICATION_ID)
      .expect("database should become index-owned");
    connection
      .execute_batch(
        "CREATE TABLE schema_migrations (
           version INTEGER PRIMARY KEY,
           checksum TEXT NOT NULL,
           applied_at_ms INTEGER NOT NULL
         );",
      )
      .expect("migration ledger should be created");
    connection
      .execute_batch(MIGRATION_001)
      .expect("original schema should be created");
    connection
      .execute(
        "INSERT INTO schema_migrations(version, checksum, applied_at_ms) VALUES(?1, ?2, ?3)",
        params![1, migration_checksum(MIGRATION_001), 0],
      )
      .expect("first migration should be recorded");
    connection
      .execute(
        "INSERT INTO sources(provider, source_key, cursor, scanned_at_ms, created_at_ms, updated_at_ms)
         VALUES(?1, ?2, ?3, ?4, ?4, ?4)",
        params![PROVIDER, SOURCE_KEY, "one", 10],
      )
      .expect("source should be inserted");
    connection
      .execute(
        "INSERT INTO sessions(
           source_id, session_id, source_path, attention_revision,
           seen_attention_revision, present, first_indexed_at_ms, indexed_at_ms
         ) VALUES(1, ?1, ?2, 0, 0, 1, 10, 10)",
        params!["a", "/sessions/a.jsonl"],
      )
      .expect("old session should be inserted");
    drop(connection);

    let index = SessionIndex::open(&path).expect("original index should migrate");
    let indexed = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "a"))
      .expect("session query should work")
      .expect("migrated session should exist");
    assert!(indexed.attention_baselined);
    assert!(!indexed.notify_on_baseline);
  }

  #[test]
  fn persists_pending_attention_baseline_state() {
    let directory = tempdir().expect("temporary directory should exist");
    let path = directory.path().join("session-index.sqlite3");
    let mut pending = session("a", None);
    assert!(pending.attention_baselined);
    assert!(!pending.notify_on_baseline);
    pending.attention_baselined = false;
    pending.notify_on_baseline = true;

    {
      let index = SessionIndex::open(&path).expect("index should open");
      index
        .replace_source(replacement("one", 10, vec![pending]))
        .expect("pending baseline should be stored");
    }

    let index = SessionIndex::open(&path).expect("index should reopen");
    let key = SessionKey::new(PROVIDER, SOURCE_KEY, "a");
    let stored_pending = index
      .session(&key)
      .expect("session query should work")
      .expect("pending session should exist");
    assert!(!stored_pending.attention_baselined);
    assert!(stored_pending.notify_on_baseline);

    let mut completed = session("a", Some("message-a"));
    completed.attention_baselined = true;
    completed.notify_on_baseline = false;
    index
      .replace_source(replacement("two", 20, vec![completed]))
      .expect("completed baseline should be stored");
    let stored_completed = index
      .session(&key)
      .expect("session query should work")
      .expect("completed session should exist");
    assert!(stored_completed.attention_baselined);
    assert!(!stored_completed.notify_on_baseline);
  }

  #[test]
  fn rejects_notify_on_an_already_established_baseline() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    let mut invalid = session("a", None);
    invalid.notify_on_baseline = true;

    assert!(matches!(
      index.replace_source(replacement("one", 10, vec![invalid])),
      Err(SessionIndexError::InvalidReplacement(message))
        if message.contains("already established attention baseline")
    ));
  }

  #[test]
  fn completing_one_pending_baseline_does_not_touch_siblings_or_increment_twice() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    let staged_source = source("catalog-one", 10);
    let completed_source = source("body-one", 11);
    let mut cataloged = pending_session("a", true);
    cataloged.title = Some("new catalog title".to_owned());
    cataloged.source_path = "/catalog/a.jsonl".to_owned();
    index
      .replace_source(SourceReplacement::new(
        staged_source.clone(),
        vec![cataloged, pending_session("b", false)],
      ))
      .expect("catalog replacement should succeed");

    let first = index
      .complete_session_baseline(
        &staged_source,
        &completed_source,
        completion_request("a", Some("message-a"), true),
      )
      .expect("first completion should run");
    assert_eq!(
      first,
      SessionBaselineCompletion::Applied {
        attention_changed: true
      }
    );
    assert!(first.was_applied());
    assert!(first.attention_changed());

    let second = index
      .complete_session_baseline(
        &staged_source,
        &source("body-two", 12),
        completion_request("a", Some("message-b"), true),
      )
      .expect("second completion should run");
    assert_eq!(second, SessionBaselineCompletion::Stale);
    assert!(!second.was_applied());
    assert!(!second.attention_changed());

    let completed = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "a"))
      .expect("session query should work")
      .expect("completed session should exist");
    assert!(completed.attention_baselined);
    assert!(!completed.notify_on_baseline);
    assert_eq!(completed.title.as_deref(), Some("new catalog title"));
    assert_eq!(completed.source_path, "/catalog/a.jsonl");
    assert_eq!(completed.attention_marker.as_deref(), Some("message-a"));
    assert_eq!(completed.attention_revision, 1);
    assert!(completed.has_unread());

    let sibling = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "b"))
      .expect("session query should work")
      .expect("sibling session should remain indexed");
    assert!(sibling.present);
    assert!(!sibling.attention_baselined);
    assert!(!sibling.has_unread());
    assert_eq!(
      index
        .source_state(&staged_source.key)
        .expect("source query should work"),
      Some(completed_source)
    );
  }

  #[test]
  fn completing_a_pending_baseline_rejects_another_source_and_stale_cursor() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    let staged_source = source("catalog-one", 10);
    let completed_source = source("body-one", 11);
    index
      .replace_source(replacement("catalog-one", 10, vec![pending_session("a", true)]))
      .expect("catalog replacement should succeed");

    assert!(matches!(
      index.complete_session_baseline(
        &staged_source,
        &staged_source,
        completion_request("a", Some("message-a"), true),
      ),
      Err(SessionIndexError::InvalidReplacement(message)) if message.contains("must advance")
    ));
    assert!(matches!(
      index.complete_session_baseline(
        &staged_source,
        &source_for(PROVIDER, "other-source", "body-one", 11),
        completion_request("a", Some("message-a"), true),
      ),
      Err(SessionIndexError::InvalidReplacement(message)) if message.contains("does not match staged source")
    ));

    let wrong_source = SessionBaselineCompletionRequest::new(
      SessionKey::new(PROVIDER, "other-source", "a"),
      Some("message-a".to_owned()),
    );
    assert!(matches!(
      index.complete_session_baseline(&staged_source, &completed_source, wrong_source),
      Err(SessionIndexError::InvalidReplacement(message))
        if message.contains("does not belong to staged source")
    ));

    let stale_source = source("catalog-two", 20);
    assert_eq!(
      index
        .complete_session_baseline(
          &stale_source,
          &source("body-two", 21),
          completion_request("a", Some("message-a"), true),
        )
        .expect("stale completion should not fail"),
      SessionBaselineCompletion::Stale
    );
    let pending = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "a"))
      .expect("session query should work")
      .expect("pending session should remain indexed");
    assert!(!pending.attention_baselined);
    assert_eq!(pending.attention_revision, 0);
  }

  #[test]
  fn baseline_completion_advancing_cursor_rejects_a_prepared_catalog_replacement() {
    let directory = tempdir().expect("temporary directory should exist");
    let path = directory.path().join("session-index.sqlite3");
    let first = SessionIndex::open(&path).expect("first index should open");
    let second = SessionIndex::open(&path).expect("second index should open");
    let staged_source = source("catalog-one", 10);
    let completed_source = source("body-one", 11);
    first
      .replace_source(SourceReplacement::new(
        staged_source.clone(),
        vec![pending_session("a", true)],
      ))
      .expect("catalog replacement should succeed");

    let prepared_catalog_replacement =
      SourceReplacement::new(source("catalog-two", 20), vec![pending_session("a", true)])
        .with_source_cursor_precondition(SourceCursorPrecondition::existing(staged_source.cursor.clone()));

    assert_eq!(
      first
        .complete_session_baseline(
          &staged_source,
          &completed_source,
          completion_request("a", Some("message-a"), true),
        )
        .expect("completion should succeed"),
      SessionBaselineCompletion::Applied {
        attention_changed: true
      }
    );

    let error = second
      .replace_source(prepared_catalog_replacement)
      .expect_err("catalog replacement prepared at the old cursor must conflict");
    assert!(matches!(
      error,
      SessionIndexError::SourceCursorConflict {
        source: conflict_source,
        expected: SourceCursorPrecondition::Exact(Some(expected)),
        actual: Some(actual),
      } if conflict_source == staged_source.key && expected == "catalog-one" && actual == "body-one"
    ));
  }

  #[test]
  fn initial_source_baseline_has_no_unread_dot() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    let result = index
      .replace_source(baseline_replacement("one", 10, vec![session("a", Some("message-a"))]))
      .expect("baseline replacement should succeed");

    assert!(result.baseline_established);
    assert_eq!(result.attention_changed, 0);
    let indexed = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "a"))
      .expect("session query should work")
      .expect("session should be indexed");
    assert_eq!(indexed.attention_revision, 0);
    assert_eq!(indexed.seen_attention_revision, 0);
    assert!(!indexed.has_unread());
    assert!(
      index
        .has_sources_for_provider(PROVIDER)
        .expect("provider query should work")
    );
  }

  #[test]
  fn caller_reported_new_attention_makes_session_unread_and_mark_seen_through_clears_it() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    index
      .replace_source(baseline_replacement("one", 10, vec![session("a", Some("message-a"))]))
      .expect("baseline replacement should succeed");
    let mut changed = session("a", Some("message-b"));
    changed.has_new_attention = true;
    let result = index
      .replace_source(replacement("two", 20, vec![changed]))
      .expect("changed marker replacement should succeed");

    assert_eq!(result.attention_changed, 1);
    let key = SessionKey::new(PROVIDER, SOURCE_KEY, "a");
    let unread = index
      .session(&key)
      .expect("session query should work")
      .expect("session should exist");
    assert_eq!(unread.attention_revision, 1);
    assert_eq!(unread.seen_attention_revision, 0);
    assert!(unread.has_unread());

    assert!(
      index
        .mark_seen_through(&key, unread.attention_revision, 30)
        .expect("mark seen should work")
    );
    let seen = index
      .session(&key)
      .expect("session query should work")
      .expect("session should exist");
    assert!(!seen.has_unread());
    assert_eq!(seen.seen_at_ms, Some(30));
  }

  #[test]
  fn track_changes_allows_a_later_discovered_source_to_be_unread() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    let mut newly_discovered = session("a", Some("message-a"));
    newly_discovered.has_new_attention = true;

    let result = index
      .replace_source(replacement("one", 10, vec![newly_discovered]))
      .expect("tracked replacement should succeed");

    assert!(!result.baseline_established);
    assert_eq!(result.attention_changed, 1);
    let indexed = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "a"))
      .expect("session query should work")
      .expect("session should exist");
    assert_eq!(indexed.attention_revision, 1);
    assert_eq!(indexed.seen_attention_revision, 0);
    assert!(indexed.has_unread());
  }

  #[test]
  fn stale_acknowledgement_cannot_clear_newer_attention() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    let key = SessionKey::new(PROVIDER, SOURCE_KEY, "a");
    index
      .replace_source(baseline_replacement("one", 10, vec![session("a", Some("message-a"))]))
      .expect("baseline replacement should succeed");

    let mut first_new_message = session("a", Some("message-b"));
    first_new_message.has_new_attention = true;
    index
      .replace_source(replacement("two", 20, vec![first_new_message]))
      .expect("first update should succeed");
    let captured_revision = index
      .session(&key)
      .expect("session query should work")
      .expect("session should exist")
      .attention_revision;

    let mut second_new_message = session("a", Some("message-c"));
    second_new_message.has_new_attention = true;
    index
      .replace_source(replacement("three", 30, vec![second_new_message]))
      .expect("second update should succeed");

    assert!(
      index
        .mark_seen_through(&key, captured_revision, 40)
        .expect("stale acknowledgement should run")
    );
    let after_stale_ack = index
      .session(&key)
      .expect("session query should work")
      .expect("session should exist");
    assert_eq!(after_stale_ack.attention_revision, 2);
    assert_eq!(after_stale_ack.seen_attention_revision, 1);
    assert!(after_stale_ack.has_unread());
    assert!(
      !index
        .mark_seen_through(&key, captured_revision, 41)
        .expect("repeat acknowledgement should not change state")
    );
    assert!(
      index
        .mark_seen_through(&key, after_stale_ack.attention_revision, 42)
        .expect("current acknowledgement should advance state")
    );
    assert!(
      !index
        .session(&key)
        .expect("session query should work")
        .expect("session should exist")
        .has_unread()
    );
  }

  #[test]
  fn metadata_or_marker_only_change_does_not_make_session_unread() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    index
      .replace_source(baseline_replacement("one", 10, vec![session("a", Some("message-a"))]))
      .expect("baseline replacement should succeed");

    let mut changed_metadata = session("a", Some("rewritten-marker"));
    changed_metadata.title = Some("A better title".to_owned());
    changed_metadata.preview = Some("A different bounded preview".to_owned());
    changed_metadata.cwd = Some("/workspace".to_owned());
    changed_metadata.updated_at = Some("another-provider-update".to_owned());
    changed_metadata.updated_at_ms = Some(2_000);
    index
      .replace_source(replacement("two", 20, vec![changed_metadata]))
      .expect("metadata replacement should succeed");

    let indexed = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "a"))
      .expect("session query should work")
      .expect("session should exist");
    assert_eq!(indexed.title.as_deref(), Some("A better title"));
    assert_eq!(indexed.preview.as_deref(), Some("A different bounded preview"));
    assert_eq!(indexed.attention_marker.as_deref(), Some("rewritten-marker"));
    assert_eq!(indexed.attention_revision, 0);
    assert_eq!(indexed.seen_attention_revision, 0);
    assert!(!indexed.has_unread());
  }

  #[test]
  fn only_a_successful_complete_replacement_tombstones_missing_sessions() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    index
      .replace_source(baseline_replacement(
        "one",
        10,
        vec![session("a", Some("message-a")), session("b", Some("message-b"))],
      ))
      .expect("baseline replacement should succeed");

    let invalid = SourceReplacement::new(
      source("failed", 20),
      vec![SessionMetadata::new(
        SessionKey::new(PROVIDER, "different-source", "a"),
        "/sessions/a.jsonl",
      )],
    );
    assert!(matches!(
      index.replace_source(invalid),
      Err(SessionIndexError::InvalidReplacement(_))
    ));
    assert_eq!(index.list_present_sessions().expect("list should work").len(), 2);

    let result = index
      .replace_source(replacement("two", 30, vec![session("a", Some("message-a"))]))
      .expect("complete replacement should succeed");
    assert_eq!(result.tombstoned, 1);
    assert_eq!(index.list_present_sessions().expect("list should work").len(), 1);
    let missing = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "b"))
      .expect("session query should work")
      .expect("tombstoned session should remain indexed");
    assert!(!missing.present);
  }

  #[test]
  fn index_persists_source_cursor_metadata_and_attention_after_reopen() {
    let directory = tempdir().expect("temporary directory should exist");
    let path = directory.path().join("session-index.sqlite3");
    {
      let index = SessionIndex::open(&path).expect("index should open");
      index
        .replace_source(baseline_replacement("one", 10, vec![session("a", Some("message-a"))]))
        .expect("baseline replacement should succeed");
      let mut changed = session("a", Some("message-b"));
      changed.has_new_attention = true;
      index
        .replace_source(replacement("two", 20, vec![changed]))
        .expect("changed marker replacement should succeed");
    }

    let index = SessionIndex::open(&path).expect("index should reopen");
    assert_eq!(
      index
        .source_state(&SourceKey::new(PROVIDER, SOURCE_KEY))
        .expect("source query should work"),
      Some(source("two", 20))
    );
    let indexed = index
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "a"))
      .expect("session query should work")
      .expect("session should persist");
    assert_eq!(indexed.source_path, "/sessions/a.jsonl");
    assert_eq!(indexed.timestamp.as_deref(), Some("2026-09-01T00:00:00Z"));
    assert_eq!(indexed.updated_at.as_deref(), Some("provider-update"));
    assert!(indexed.has_unread());
  }

  #[test]
  fn source_cursor_preconditions_reject_a_stale_writer_without_tombstoning_newer_rows() {
    let directory = tempdir().expect("temporary directory should exist");
    let path = directory.path().join("session-index.sqlite3");
    let first_writer = SessionIndex::open(&path).expect("first index should open");
    let second_writer = SessionIndex::open(&path).expect("second index should open");
    let source_key = SourceKey::new(PROVIDER, SOURCE_KEY);

    let initial_snapshot = first_writer
      .source_state(&source_key)
      .expect("source query should work");
    first_writer
      .replace_source(
        baseline_replacement("one", 10, vec![session("a", Some("message-a"))]).with_source_cursor_precondition(
          SourceCursorPrecondition::Exact(initial_snapshot.map(|state| state.cursor)),
        ),
      )
      .expect("an absent-source precondition should match an absent source");

    let stale_snapshot = first_writer
      .source_state(&source_key)
      .expect("source query should work")
      .expect("source should be indexed");
    let mut newer_session = session("a", Some("message-b"));
    newer_session.title = Some("newer writer title".to_owned());
    second_writer
      .replace_source(
        replacement("two", 20, vec![newer_session])
          .with_source_cursor_precondition(SourceCursorPrecondition::existing(stale_snapshot.cursor.clone())),
      )
      .expect("the current cursor should satisfy the second writer");

    let stale_replacement = replacement("three", 30, vec![session("a", Some("message-c"))])
      .with_source_cursor_precondition(SourceCursorPrecondition::existing(stale_snapshot.cursor));
    let error = first_writer
      .replace_source(stale_replacement)
      .expect_err("a stale source cursor must reject the replacement");
    assert!(matches!(
      error,
      SessionIndexError::SourceCursorConflict {
        source,
        expected: SourceCursorPrecondition::Exact(Some(expected)),
        actual: Some(actual),
      } if source == source_key && expected == "one" && actual == "two"
    ));

    assert_eq!(
      first_writer
        .source_state(&source_key)
        .expect("source query should work"),
      Some(source("two", 20))
    );
    let current = first_writer
      .session(&SessionKey::new(PROVIDER, SOURCE_KEY, "a"))
      .expect("session query should work")
      .expect("newer row should remain indexed");
    assert_eq!(current.title.as_deref(), Some("newer writer title"));
  }

  #[test]
  fn batch_replacement_keeps_every_source_unchanged_when_a_later_precondition_is_stale() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    let first_source_key = "source-a";
    let second_source_key = "source-b";
    let first_key = SourceKey::new(PROVIDER, first_source_key);
    let second_key = SourceKey::new(PROVIDER, second_source_key);

    index
      .replace_sources(&[
        SourceReplacement::baseline(
          source_for(PROVIDER, first_source_key, "one", 10),
          vec![
            session_for(PROVIDER, first_source_key, "keep", Some("message-keep")),
            session_for(PROVIDER, first_source_key, "removed", Some("message-removed")),
          ],
        ),
        SourceReplacement::baseline(
          source_for(PROVIDER, second_source_key, "one", 10),
          vec![session_for(PROVIDER, second_source_key, "other", Some("message-other"))],
        ),
      ])
      .expect("initial batch should succeed");

    let mut current_second_session = session_for(PROVIDER, second_source_key, "other", Some("message-current"));
    current_second_session.title = Some("current second title".to_owned());
    index
      .replace_source(
        SourceReplacement::new(
          source_for(PROVIDER, second_source_key, "two", 20),
          vec![current_second_session],
        )
        .with_source_cursor_precondition(SourceCursorPrecondition::existing("one")),
      )
      .expect("second source should advance before the stale batch");

    let mut first_pending_session = session_for(PROVIDER, first_source_key, "keep", Some("message-new"));
    first_pending_session.title = Some("must not commit".to_owned());
    first_pending_session.has_new_attention = true;
    let stale_batch = [
      SourceReplacement::new(
        source_for(PROVIDER, first_source_key, "two", 30),
        vec![first_pending_session],
      )
      .with_source_cursor_precondition(SourceCursorPrecondition::existing("one")),
      SourceReplacement::new(
        source_for(PROVIDER, second_source_key, "three", 30),
        vec![session_for(PROVIDER, second_source_key, "other", Some("message-stale"))],
      )
      .with_source_cursor_precondition(SourceCursorPrecondition::existing("one")),
    ];

    let error = index
      .replace_sources(&stale_batch)
      .expect_err("a stale later source must reject the whole batch");
    assert!(matches!(
      error,
      SessionIndexError::SourceCursorConflict {
        source,
        expected: SourceCursorPrecondition::Exact(Some(expected)),
        actual: Some(actual),
      } if source == second_key && expected == "one" && actual == "two"
    ));

    assert_eq!(
      index.source_state(&first_key).expect("first source query should work"),
      Some(source_for(PROVIDER, first_source_key, "one", 10))
    );
    let retained = index
      .session(&SessionKey::new(PROVIDER, first_source_key, "keep"))
      .expect("first session query should work")
      .expect("first session should remain indexed");
    assert_eq!(retained.title.as_deref(), Some("Title keep"));
    assert_eq!(retained.attention_revision, 0);
    assert!(
      index
        .session(&SessionKey::new(PROVIDER, first_source_key, "removed"))
        .expect("tombstoned session query should work")
        .expect("omitted session should remain indexed")
        .present
    );
    assert_eq!(
      index
        .source_state(&second_key)
        .expect("second source query should work"),
      Some(source_for(PROVIDER, second_source_key, "two", 20))
    );
  }

  #[test]
  fn list_order_is_total_when_sessions_share_update_time_and_id() {
    let index = SessionIndex::open_in_memory().expect("index should open");
    for (provider, source_key) in [("codex", "source-b"), ("codex", "source-a"), ("pi", "source-a")] {
      index
        .replace_source(SourceReplacement::baseline(
          source_for(provider, source_key, "one", 10),
          vec![session_for(provider, source_key, "shared-session", Some("message"))],
        ))
        .expect("replacement should succeed");
    }

    let keys = index
      .list_present_sessions()
      .expect("list should work")
      .into_iter()
      .map(|session| (session.key.provider, session.key.source_key, session.key.session_id))
      .collect::<Vec<_>>();
    assert_eq!(
      keys,
      vec![
        ("codex".to_owned(), "source-a".to_owned(), "shared-session".to_owned()),
        ("codex".to_owned(), "source-b".to_owned(), "shared-session".to_owned()),
        ("pi".to_owned(), "source-a".to_owned(), "shared-session".to_owned()),
      ]
    );
  }

  #[test]
  fn migration_checksum_drift_is_rejected() {
    let directory = tempdir().expect("temporary directory should exist");
    let path = directory.path().join("session-index.sqlite3");
    let index = SessionIndex::open(&path).expect("index should open");
    drop(index);

    let connection = Connection::open(&path).expect("database should reopen");
    connection
      .execute("UPDATE schema_migrations SET checksum = 'bad' WHERE version = 1", [])
      .expect("migration checksum should update for test");
    drop(connection);

    assert!(matches!(
      SessionIndex::open(&path),
      Err(SessionIndexError::MigrationChecksumMismatch { version: 1, .. })
    ));
  }

  #[test]
  fn refuses_to_migrate_an_unowned_existing_database() {
    let directory = tempdir().expect("temporary directory should exist");
    let path = directory.path().join("unowned.sqlite3");
    let connection = Connection::open(&path).expect("database should open");
    connection
      .execute("CREATE TABLE unrelated_cache(id INTEGER PRIMARY KEY)", [])
      .expect("unrelated schema should be created");
    let journal_before: String = connection
      .pragma_query_value(None, "journal_mode", |row| row.get(0))
      .expect("journal mode should be readable");
    drop(connection);

    assert!(matches!(
      SessionIndex::open(&path),
      Err(SessionIndexError::UnexpectedDatabase(0))
    ));

    let connection = Connection::open(&path).expect("database should reopen");
    let journal_after: String = connection
      .pragma_query_value(None, "journal_mode", |row| row.get(0))
      .expect("journal mode should remain readable");
    assert_eq!(journal_after, journal_before);
  }
}
