//! Passwordless owner login. WebAuthn ceremony state and bearer sessions never
//! leave this process; only verified public credentials are persisted.

use std::{
  collections::HashMap,
  sync::{Arc, Mutex, MutexGuard},
  time::{Duration, Instant},
};

use axum::{
  Json, Router,
  extract::{DefaultBodyLimit, State},
  http::{HeaderMap, HeaderValue, StatusCode, header},
  middleware,
  response::{IntoResponse, Response},
  routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;
use url::Url;
use webauthn_rs::prelude::*;

use crate::store::Store;

const CEREMONY_TTL: Duration = Duration::from_secs(300);
const SESSION_TTL: Duration = Duration::from_secs(3600);
const MAX_REGISTRATION_CEREMONIES: usize = 32;
const MAX_AUTHENTICATION_CEREMONIES: usize = 128;
const MAX_SESSIONS: usize = 128;

type TokenHash = [u8; 32];

#[derive(Clone)]
pub struct AuthState {
  inner: Arc<AuthInner>,
}

struct AuthInner {
  store: Store,
  webauthn: Webauthn,
  owner_id: Uuid,
  origin: String,
  memory: Mutex<AuthMemory>,
}

#[derive(Default)]
struct AuthMemory {
  closed: bool,
  bootstrap_hash: Option<TokenHash>,
  bootstrap_display: Option<String>,
  ceremonies: HashMap<String, PendingCeremony>,
  sessions: HashMap<TokenHash, Session>,
}

struct Session {
  expires_at: Instant,
  cancellation: CancellationToken,
}

struct PendingCeremony {
  expires_at: Instant,
  ceremony: Ceremony,
}

enum Ceremony {
  Registration {
    state: PasskeyRegistration,
    authorization: RegistrationAuthorization,
  },
  Authentication(PasskeyAuthentication),
}

enum RegistrationAuthorization {
  Bootstrap,
  Session(TokenHash),
}

/// This first Hub version has one owner. Host approvals separately limit the
/// operations that owner can perform through a particular connector.
#[derive(Clone, Debug)]
pub struct Principal {
  pub user_id: String,
  /// Cancel proxied work when the session expires or is logged out.
  pub cancellation: CancellationToken,
}

#[derive(Debug)]
pub struct AuthError {
  pub status: StatusCode,
  message: String,
}

impl AuthError {
  fn new(status: StatusCode, message: impl Into<String>) -> Self {
    Self {
      status,
      message: message.into(),
    }
  }

  fn unauthorized() -> Self {
    Self::new(StatusCode::UNAUTHORIZED, "Sign in to the Hub with a passkey")
  }

  fn internal(error: String) -> Self {
    eprintln!("Hub authentication: {error}");
    Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Hub authentication is unavailable")
  }
}

impl IntoResponse for AuthError {
  fn into_response(self) -> Response {
    (self.status, Json(serde_json::json!({"error": self.message}))).into_response()
  }
}

impl AuthState {
  pub fn new(store: Store, public_url: &str) -> Result<Self, String> {
    let origin_url = validate_public_url(public_url)?;
    let rp_id = origin_url.domain().ok_or("Hub public URL must use a domain name")?;
    let webauthn = WebauthnBuilder::new(rp_id, &origin_url)
      .map_err(|error| format!("Invalid passkey origin: {error}"))?
      .rp_name("Tokn Hub")
      .timeout(CEREMONY_TTL)
      .build()
      .map_err(|error| format!("Configure passkeys: {error}"))?;
    let mut memory = AuthMemory::default();
    if !store.configured()? {
      let token = random_token();
      memory.bootstrap_hash = Some(token_hash(&token));
      memory.bootstrap_display = Some(token);
    }
    Ok(Self {
      inner: Arc::new(AuthInner {
        owner_id: store.owner_id()?,
        store,
        webauthn,
        origin: origin_url.origin().ascii_serialization(),
        memory: Mutex::new(memory),
      }),
    })
  }

  /// Returns the initial setup secret once for the local server operator. It is
  /// invalidated by successful setup and is never persisted or exposed by HTTP.
  pub fn bootstrap_token(&self) -> Result<Option<String>, String> {
    Ok(
      self
        .inner
        .memory
        .lock()
        .map_err(|_| "Hub authentication lock poisoned")?
        .bootstrap_display
        .take(),
    )
  }

  pub fn router(&self) -> Router {
    Router::new()
      .route("/hub/v1/auth/status", get(status))
      .route("/hub/v1/auth/register/start", post(register_start))
      .route("/hub/v1/auth/register/finish", post(register_finish))
      .route("/hub/v1/auth/login/start", post(login_start))
      .route("/hub/v1/auth/login/finish", post(login_finish))
      .route("/hub/v1/auth/logout", post(logout))
      .layer(DefaultBodyLimit::max(64 * 1024))
      .layer(middleware::map_response(no_store))
      .with_state(self.clone())
  }

  pub fn authorize(&self, headers: &HeaderMap) -> Result<Principal, AuthError> {
    let hash = bearer_hash(headers)?;
    let mut memory = self.memory()?;
    memory.prune(Instant::now());
    let session = memory.sessions.get(&hash).ok_or_else(AuthError::unauthorized)?;
    Ok(Principal {
      user_id: self.inner.owner_id.to_string(),
      cancellation: session.cancellation.clone(),
    })
  }

  /// Browser authentication mutations always require the configured origin.
  pub fn check_origin(&self, headers: &HeaderMap) -> Result<(), AuthError> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let origin = origins.next().and_then(|value| value.to_str().ok());
    if origin != Some(self.inner.origin.as_str()) || origins.next().is_some() {
      return Err(AuthError::new(
        StatusCode::FORBIDDEN,
        "Request origin does not match the Hub",
      ));
    }
    Ok(())
  }

  /// Authenticated non-browser API clients may omit Origin. Browsers may not
  /// substitute a different origin when issuing host-control requests.
  pub fn check_optional_origin(&self, headers: &HeaderMap) -> Result<(), AuthError> {
    if headers.contains_key(header::ORIGIN) {
      self.check_origin(headers)?;
    }
    Ok(())
  }

  pub fn shutdown(&self) {
    if let Ok(mut memory) = self.inner.memory.lock() {
      memory.closed = true;
      for (_, session) in memory.sessions.drain() {
        session.cancellation.cancel();
      }
      memory.ceremonies.clear();
      memory.bootstrap_hash = None;
      memory.bootstrap_display = None;
    }
  }

  fn memory(&self) -> Result<MutexGuard<'_, AuthMemory>, AuthError> {
    let memory = self
      .inner
      .memory
      .lock()
      .map_err(|_| AuthError::internal("Authentication lock poisoned".to_owned()))?;
    if memory.closed {
      return Err(AuthError::new(StatusCode::SERVICE_UNAVAILABLE, "Hub is shutting down"));
    }
    Ok(memory)
  }

  fn insert_ceremony(&self, ceremony: Ceremony) -> Result<String, AuthError> {
    let mut memory = self.memory()?;
    memory.prune(Instant::now());
    let registration_count = memory
      .ceremonies
      .values()
      .filter(|pending| matches!(&pending.ceremony, Ceremony::Registration { .. }))
      .count();
    if matches!(&ceremony, Ceremony::Registration { .. }) {
      // Registration is authenticated by the bootstrap secret or owner session.
      // Anonymous login traffic must never consume this reserved capacity.
      if registration_count >= MAX_REGISTRATION_CEREMONIES {
        return Err(AuthError::new(
          StatusCode::TOO_MANY_REQUESTS,
          "Too many pending passkey registrations; try again shortly",
        ));
      }
    } else if memory.ceremonies.len() - registration_count >= MAX_AUTHENTICATION_CEREMONIES {
      // Do not lock everyone out until a full set of abandoned login attempts
      // expires. Admit the new attempt by retiring the oldest anonymous one.
      // Internet-facing deployments still need endpoint rate limiting.
      let oldest = memory
        .ceremonies
        .iter()
        .filter(|(_, pending)| matches!(&pending.ceremony, Ceremony::Authentication(_)))
        .min_by_key(|(_, pending)| pending.expires_at)
        .map(|(id, _)| id.clone());
      if let Some(oldest) = oldest {
        memory.ceremonies.remove(&oldest);
      }
    }
    let ceremony_id = random_token();
    memory.ceremonies.insert(
      ceremony_id.clone(),
      PendingCeremony {
        expires_at: Instant::now() + CEREMONY_TTL,
        ceremony,
      },
    );
    Ok(ceremony_id)
  }

  fn take_ceremony(&self, ceremony_id: &str) -> Result<Ceremony, AuthError> {
    let mut memory = self.memory()?;
    memory.prune(Instant::now());
    memory
      .ceremonies
      .remove(ceremony_id)
      .map(|pending| pending.ceremony)
      .ok_or_else(|| {
        AuthError::new(
          StatusCode::BAD_REQUEST,
          "Passkey request expired or was already used; start again",
        )
      })
  }

  fn issue_session(&self) -> Result<TokenResponse, AuthError> {
    self.issue_session_with_ttl(SESSION_TTL)
  }

  fn issue_session_with_ttl(&self, ttl: Duration) -> Result<TokenResponse, AuthError> {
    let access_token = random_token();
    let hash = token_hash(&access_token);
    let cancellation = CancellationToken::new();
    let expires_at = Instant::now() + ttl;
    {
      let mut memory = self.memory()?;
      memory.prune(Instant::now());
      if memory.sessions.len() >= MAX_SESSIONS {
        return Err(AuthError::new(
          StatusCode::TOO_MANY_REQUESTS,
          "Too many active Hub sessions",
        ));
      }
      memory.sessions.insert(
        hash,
        Session {
          expires_at,
          cancellation: cancellation.clone(),
        },
      );
    }
    let weak = Arc::downgrade(&self.inner);
    tokio::spawn(async move {
      tokio::select! {
        _ = tokio::time::sleep_until(expires_at.into()) => cancellation.cancel(),
        _ = cancellation.cancelled() => {},
      }
      if let Some(inner) = weak.upgrade()
        && let Ok(mut memory) = inner.memory.lock()
      {
        memory.sessions.remove(&hash);
      }
    });
    Ok(TokenResponse {
      access_token,
      token_type: "Bearer",
      expires_in: ttl.as_secs(),
    })
  }
}

impl AuthMemory {
  fn prune(&mut self, now: Instant) {
    self.ceremonies.retain(|_, pending| pending.expires_at > now);
    self.sessions.retain(|_, session| {
      if session.expires_at <= now || session.cancellation.is_cancelled() {
        session.cancellation.cancel();
        false
      } else {
        true
      }
    });
  }
}

impl Drop for AuthInner {
  fn drop(&mut self) {
    if let Ok(memory) = self.memory.get_mut() {
      for session in memory.sessions.values() {
        session.cancellation.cancel();
      }
    }
  }
}

#[derive(Serialize)]
struct AuthStatus {
  configured: bool,
  authenticated: bool,
}

#[derive(Deserialize)]
struct RegistrationStart {
  bootstrap_token: Option<String>,
}

#[derive(Serialize)]
struct ChallengeResponse<T> {
  ceremony_id: String,
  options: T,
}

#[derive(Deserialize)]
struct RegistrationFinish {
  ceremony_id: String,
  credential: RegisterPublicKeyCredential,
}

#[derive(Deserialize)]
struct AuthenticationFinish {
  ceremony_id: String,
  credential: PublicKeyCredential,
}

#[derive(Debug, Serialize)]
struct TokenResponse {
  access_token: String,
  token_type: &'static str,
  expires_in: u64,
}

async fn no_store(mut response: Response) -> Response {
  response
    .headers_mut()
    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
  response
}

async fn status(State(auth): State<AuthState>, headers: HeaderMap) -> Result<Json<AuthStatus>, AuthError> {
  Ok(Json(AuthStatus {
    configured: auth.inner.store.configured().map_err(AuthError::internal)?,
    authenticated: auth.authorize(&headers).is_ok(),
  }))
}

async fn register_start(
  State(auth): State<AuthState>,
  headers: HeaderMap,
  Json(request): Json<RegistrationStart>,
) -> Result<Json<ChallengeResponse<CreationChallengeResponse>>, AuthError> {
  auth.check_origin(&headers)?;
  let passkeys = auth.inner.store.passkeys().map_err(AuthError::internal)?;
  let authorization = if passkeys.is_empty() {
    let supplied = request.bootstrap_token.as_deref().ok_or_else(AuthError::unauthorized)?;
    let memory = auth.memory()?;
    let valid = memory
      .bootstrap_hash
      .as_ref()
      .is_some_and(|expected| expected.ct_eq(&token_hash(supplied)).into());
    if !valid {
      return Err(AuthError::unauthorized());
    }
    RegistrationAuthorization::Bootstrap
  } else {
    auth.authorize(&headers)?;
    RegistrationAuthorization::Session(bearer_hash(&headers)?)
  };
  let excluded = passkeys.iter().map(|passkey| passkey.cred_id().clone()).collect();
  let (options, state) = auth
    .inner
    .webauthn
    .start_passkey_registration(auth.inner.owner_id, "owner", "Hub owner", Some(excluded))
    .map_err(|error| AuthError::internal(format!("Start registration: {error}")))?;
  let ceremony_id = auth.insert_ceremony(Ceremony::Registration { state, authorization })?;
  Ok(Json(ChallengeResponse { ceremony_id, options }))
}

async fn register_finish(
  State(auth): State<AuthState>,
  headers: HeaderMap,
  Json(request): Json<RegistrationFinish>,
) -> Result<Json<TokenResponse>, AuthError> {
  auth.check_origin(&headers)?;
  let Ceremony::Registration { state, authorization } = auth.take_ceremony(&request.ceremony_id)? else {
    return Err(AuthError::new(
      StatusCode::BAD_REQUEST,
      "Expected a registration request",
    ));
  };
  let first = match authorization {
    RegistrationAuthorization::Bootstrap => true,
    RegistrationAuthorization::Session(expected) => {
      auth.authorize(&headers)?;
      if expected != bearer_hash(&headers)? {
        return Err(AuthError::unauthorized());
      }
      false
    }
  };
  let passkey = auth
    .inner
    .webauthn
    .finish_passkey_registration(&request.credential, &state)
    .map_err(|_| AuthError::new(StatusCode::BAD_REQUEST, "Passkey registration could not be verified"))?;
  auth.inner.store.register_passkey(&passkey, first).map_err(|error| {
    eprintln!("Hub registration: {error}");
    AuthError::new(StatusCode::CONFLICT, "Passkey could not be added; restart registration")
  })?;
  if first {
    let mut memory = auth.memory()?;
    memory.bootstrap_hash = None;
    memory.bootstrap_display = None;
    memory.ceremonies.retain(|_, pending| {
      !matches!(
        pending.ceremony,
        Ceremony::Registration {
          authorization: RegistrationAuthorization::Bootstrap,
          ..
        }
      )
    });
  }
  Ok(Json(auth.issue_session()?))
}

async fn login_start(
  State(auth): State<AuthState>,
  headers: HeaderMap,
) -> Result<Json<ChallengeResponse<RequestChallengeResponse>>, AuthError> {
  auth.check_origin(&headers)?;
  let passkeys = auth.inner.store.passkeys().map_err(AuthError::internal)?;
  if passkeys.is_empty() {
    return Err(AuthError::new(StatusCode::CONFLICT, "Complete Hub setup first"));
  }
  let (options, state) = auth
    .inner
    .webauthn
    .start_passkey_authentication(&passkeys)
    .map_err(|error| AuthError::internal(format!("Start login: {error}")))?;
  let ceremony_id = auth.insert_ceremony(Ceremony::Authentication(state))?;
  Ok(Json(ChallengeResponse { ceremony_id, options }))
}

async fn login_finish(
  State(auth): State<AuthState>,
  headers: HeaderMap,
  Json(request): Json<AuthenticationFinish>,
) -> Result<Json<TokenResponse>, AuthError> {
  auth.check_origin(&headers)?;
  let Ceremony::Authentication(state) = auth.take_ceremony(&request.ceremony_id)? else {
    return Err(AuthError::new(
      StatusCode::BAD_REQUEST,
      "Expected an authentication request",
    ));
  };
  let result = auth
    .inner
    .webauthn
    .finish_passkey_authentication(&request.credential, &state)
    .map_err(|_| AuthError::new(StatusCode::UNAUTHORIZED, "Passkey login could not be verified"))?;
  auth.inner.store.update_passkey(&result).map_err(AuthError::internal)?;
  Ok(Json(auth.issue_session()?))
}

async fn logout(State(auth): State<AuthState>, headers: HeaderMap) -> Result<StatusCode, AuthError> {
  auth.check_origin(&headers)?;
  let hash = bearer_hash(&headers)?;
  if let Some(session) = auth.memory()?.sessions.remove(&hash) {
    session.cancellation.cancel();
  }
  Ok(StatusCode::NO_CONTENT)
}

fn validate_public_url(value: &str) -> Result<Url, String> {
  let url = Url::parse(value).map_err(|error| format!("Invalid Hub public URL: {error}"))?;
  let domain = url
    .domain()
    .ok_or("Hub public URL must use a domain name; use localhost for local development")?;
  if !url.username().is_empty()
    || url.password().is_some()
    || url.query().is_some()
    || url.fragment().is_some()
    || url.path() != "/"
  {
    return Err("Hub public URL must be an origin with no credentials, path, query, or fragment".to_owned());
  }
  if url.scheme() != "https" && !(url.scheme() == "http" && domain == "localhost") {
    return Err("Hub public URL requires HTTPS (HTTP is only allowed for localhost development)".to_owned());
  }
  Ok(url)
}

fn random_token() -> String {
  let mut bytes = [0u8; 32];
  OsRng.fill_bytes(&mut bytes);
  URL_SAFE_NO_PAD.encode(bytes)
}

fn token_hash(token: &str) -> TokenHash {
  Sha256::digest(token.as_bytes()).into()
}

fn bearer_hash(headers: &HeaderMap) -> Result<TokenHash, AuthError> {
  let mut values = headers.get_all(header::AUTHORIZATION).iter();
  let value = values
    .next()
    .and_then(|value| value.to_str().ok())
    .ok_or_else(AuthError::unauthorized)?;
  if values.next().is_some() {
    return Err(AuthError::unauthorized());
  }
  let (scheme, token) = value.split_once(' ').ok_or_else(AuthError::unauthorized)?;
  if !scheme.eq_ignore_ascii_case("Bearer")
    || token.len() != 43
    || !token
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
  {
    return Err(AuthError::unauthorized());
  }
  Ok(token_hash(token))
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
