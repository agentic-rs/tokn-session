use serde_json::{Value, json};
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};

use super::{test_support::*, *};

const ORIGIN: &str = "https://hub.example.com";

fn setup() -> (tempfile::TempDir, AuthState) {
  let directory = tempfile::tempdir().unwrap();
  let store = Store::open(directory.path().join("hub.sqlite3")).unwrap();
  let auth = AuthState::new(store, ORIGIN).unwrap();
  (directory, auth)
}

fn headers(token: &str) -> HeaderMap {
  let mut headers = HeaderMap::new();
  headers.insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
  headers.insert(header::ORIGIN, ORIGIN.parse().unwrap());
  headers
}

async fn registration(
  auth: &AuthState,
  authenticator: &mut WebauthnAuthenticator<SoftPasskey>,
  token: Option<&str>,
  bootstrap: Option<&str>,
) -> Value {
  let (status, start) = post_json(
    auth.router(),
    "/hub/v1/auth/register/start",
    Some(ORIGIN),
    token,
    json!({"bootstrap_token": bootstrap}),
  )
  .await;
  assert_eq!(status, StatusCode::OK, "{start}");
  let credential = authenticator
    .do_registration(
      Url::parse(ORIGIN).unwrap(),
      serde_json::from_value(start["options"].clone()).unwrap(),
    )
    .unwrap();
  json!({"ceremony_id": start["ceremony_id"], "credential": credential})
}

async fn login(auth: &AuthState, authenticator: &mut WebauthnAuthenticator<SoftPasskey>) -> Value {
  let (status, start) = post_json(auth.router(), "/hub/v1/auth/login/start", Some(ORIGIN), None, json!({})).await;
  assert_eq!(status, StatusCode::OK, "{start}");
  let credential = authenticator
    .do_authentication(
      Url::parse(ORIGIN).unwrap(),
      serde_json::from_value(start["options"].clone()).unwrap(),
    )
    .unwrap();
  json!({"ceremony_id": start["ceremony_id"], "credential": credential})
}

#[tokio::test]
async fn owner_registration_login_and_additional_passkey_are_verified() {
  let (directory, auth) = setup();
  let bootstrap = auth.bootstrap_token().unwrap().unwrap();
  assert!(auth.bootstrap_token().unwrap().is_none());
  let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));
  let registration_body = registration(&auth, &mut authenticator, None, Some(&bootstrap)).await;
  let (status, response) = post_json(
    auth.router(),
    "/hub/v1/auth/register/finish",
    Some(ORIGIN),
    None,
    registration_body.clone(),
  )
  .await;
  assert_eq!(status, StatusCode::OK, "{response}");
  let token = response["access_token"].as_str().unwrap();
  assert!(auth.authorize(&headers(token)).is_ok());
  assert_eq!(response["expires_in"], 3600);
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/register/finish",
      Some(ORIGIN),
      None,
      registration_body
    )
    .await
    .0,
    StatusCode::BAD_REQUEST
  );
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/register/start",
      Some(ORIGIN),
      None,
      json!({"bootstrap_token": bootstrap})
    )
    .await
    .0,
    StatusCode::UNAUTHORIZED
  );

  let request = login(&auth, &mut authenticator).await;
  let (status, response) = post_json(
    auth.router(),
    "/hub/v1/auth/login/finish",
    Some(ORIGIN),
    None,
    request.clone(),
  )
  .await;
  assert_eq!(status, StatusCode::OK, "{response}");
  assert_eq!(
    post_json(auth.router(), "/hub/v1/auth/login/finish", Some(ORIGIN), None, request)
      .await
      .0,
    StatusCode::BAD_REQUEST
  );

  let mut second = WebauthnAuthenticator::new(SoftPasskey::new(true));
  let request = registration(&auth, &mut second, Some(token), None).await;
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/register/finish",
      Some(ORIGIN),
      Some(token),
      request
    )
    .await
    .0,
    StatusCode::OK
  );
  assert_eq!(auth.inner.store.passkeys().unwrap().len(), 2);
  let restarted = AuthState::new(Store::open(directory.path().join("hub.sqlite3")).unwrap(), ORIGIN).unwrap();
  assert!(restarted.bootstrap_token().unwrap().is_none());
  assert!(restarted.authorize(&headers(token)).is_err());
  let request = login(&restarted, &mut second).await;
  assert_eq!(
    post_json(
      restarted.router(),
      "/hub/v1/auth/login/finish",
      Some(ORIGIN),
      None,
      request
    )
    .await
    .0,
    StatusCode::OK
  );
}

#[tokio::test]
async fn two_bootstrap_ceremonies_cannot_claim_the_same_installation() {
  let (directory, first) = setup();
  // Distinct Store/AuthState instances exercise SQLite's atomic claim, not
  // just the ceremony cleanup performed by one process's in-memory state.
  let second = AuthState::new(Store::open(directory.path().join("hub.sqlite3")).unwrap(), ORIGIN).unwrap();
  let first_secret = first.bootstrap_token().unwrap().unwrap();
  let second_secret = second.bootstrap_token().unwrap().unwrap();
  let first_request = registration(
    &first,
    &mut WebauthnAuthenticator::new(SoftPasskey::new(true)),
    None,
    Some(&first_secret),
  )
  .await;
  let second_request = registration(
    &second,
    &mut WebauthnAuthenticator::new(SoftPasskey::new(true)),
    None,
    Some(&second_secret),
  )
  .await;
  let (left, right) = tokio::join!(
    post_json(
      first.router(),
      "/hub/v1/auth/register/finish",
      Some(ORIGIN),
      None,
      first_request
    ),
    post_json(
      second.router(),
      "/hub/v1/auth/register/finish",
      Some(ORIGIN),
      None,
      second_request
    ),
  );
  assert_eq!(
    [left.0, right.0]
      .into_iter()
      .filter(|status| *status == StatusCode::OK)
      .count(),
    1
  );
  assert_eq!(first.inner.store.passkeys().unwrap().len(), 1);
}

#[tokio::test]
async fn expired_or_invalid_authentication_is_consumed_and_cannot_be_replayed() {
  let (_directory, auth) = setup();
  let secret = auth.bootstrap_token().unwrap().unwrap();
  let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));
  let request = registration(&auth, &mut authenticator, None, Some(&secret)).await;
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/register/finish",
      Some(ORIGIN),
      None,
      request
    )
    .await
    .0,
    StatusCode::OK
  );
  let request = login(&auth, &mut authenticator).await;
  auth
    .memory()
    .unwrap()
    .ceremonies
    .get_mut(request["ceremony_id"].as_str().unwrap())
    .unwrap()
    .expires_at = Instant::now();
  assert_eq!(
    post_json(auth.router(), "/hub/v1/auth/login/finish", Some(ORIGIN), None, request)
      .await
      .0,
    StatusCode::BAD_REQUEST
  );
  let request = login(&auth, &mut authenticator).await;
  let mut tampered = request.clone();
  tampered["credential"]["response"]["signature"] = json!("AA");
  assert_eq!(
    post_json(auth.router(), "/hub/v1/auth/login/finish", Some(ORIGIN), None, tampered)
      .await
      .0,
    StatusCode::UNAUTHORIZED
  );
  assert_eq!(
    post_json(auth.router(), "/hub/v1/auth/login/finish", Some(ORIGIN), None, request)
      .await
      .0,
    StatusCode::BAD_REQUEST
  );

  // A correctly signed assertion for a different origin is still rejected,
  // even when a caller supplies the expected HTTP Origin header.
  let (status, start) = post_json(auth.router(), "/hub/v1/auth/login/start", Some(ORIGIN), None, json!({})).await;
  assert_eq!(status, StatusCode::OK);
  let credential = authenticator
    .do_authentication(
      Url::parse("https://hub.example.com:444").unwrap(),
      serde_json::from_value(start["options"].clone()).unwrap(),
    )
    .unwrap();
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/login/finish",
      Some(ORIGIN),
      None,
      json!({"ceremony_id": start["ceremony_id"], "credential": credential})
    )
    .await
    .0,
    StatusCode::UNAUTHORIZED
  );
}

#[tokio::test]
async fn pending_additional_passkey_requires_its_original_active_session() {
  let (_directory, auth) = setup();
  let token = login_for_test(auth.router(), &auth).await;
  let request = registration(
    &auth,
    &mut WebauthnAuthenticator::new(SoftPasskey::new(true)),
    Some(&token),
    None,
  )
  .await;
  let other_token = auth.issue_session().unwrap().access_token;
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/register/finish",
      Some(ORIGIN),
      Some(&other_token),
      request
    )
    .await
    .0,
    StatusCode::UNAUTHORIZED
  );
  let request = registration(
    &auth,
    &mut WebauthnAuthenticator::new(SoftPasskey::new(true)),
    Some(&token),
    None,
  )
  .await;
  post_json(
    auth.router(),
    "/hub/v1/auth/logout",
    Some(ORIGIN),
    Some(&token),
    json!({}),
  )
  .await;
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/register/finish",
      Some(ORIGIN),
      Some(&token),
      request
    )
    .await
    .0,
    StatusCode::UNAUTHORIZED
  );
  assert_eq!(auth.inner.store.passkeys().unwrap().len(), 1);
}

#[tokio::test]
async fn exact_origin_and_bootstrap_are_required_and_ceremonies_are_bounded() {
  let (_directory, auth) = setup();
  let secret = auth.bootstrap_token().unwrap().unwrap();
  for origin in [
    None,
    Some("https://evil.example.com"),
    Some("https://hub.example.com:444"),
  ] {
    assert_eq!(
      post_json(
        auth.router(),
        "/hub/v1/auth/register/start",
        origin,
        None,
        json!({"bootstrap_token": secret})
      )
      .await
      .0,
      StatusCode::FORBIDDEN
    );
  }
  for body in [json!({}), json!({"bootstrap_token":"wrong"})] {
    assert_eq!(
      post_json(auth.router(), "/hub/v1/auth/register/start", Some(ORIGIN), None, body)
        .await
        .0,
      StatusCode::UNAUTHORIZED
    );
  }
  for _ in 0..MAX_REGISTRATION_CEREMONIES {
    assert_eq!(
      post_json(
        auth.router(),
        "/hub/v1/auth/register/start",
        Some(ORIGIN),
        None,
        json!({"bootstrap_token": secret})
      )
      .await
      .0,
      StatusCode::OK
    );
  }
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/register/start",
      Some(ORIGIN),
      None,
      json!({"bootstrap_token": secret})
    )
    .await
    .0,
    StatusCode::TOO_MANY_REQUESTS
  );
}

#[tokio::test]
async fn logout_expiry_and_shutdown_cancel_active_principals() {
  let (_directory, auth) = setup();
  let token = login_for_test(auth.router(), &auth).await;
  let principal = auth.authorize(&headers(&token)).unwrap();
  assert_eq!(
    post_json(
      auth.router(),
      "/hub/v1/auth/logout",
      Some(ORIGIN),
      Some(&token),
      json!({})
    )
    .await
    .0,
    StatusCode::NO_CONTENT
  );
  assert!(principal.cancellation.is_cancelled());
  assert!(auth.authorize(&headers(&token)).is_err());
  let token = auth
    .issue_session_with_ttl(Duration::from_millis(20))
    .unwrap()
    .access_token;
  let principal = auth.authorize(&headers(&token)).unwrap();
  tokio::time::timeout(Duration::from_secs(1), principal.cancellation.cancelled())
    .await
    .unwrap();
  assert!(auth.authorize(&headers(&token)).is_err());
  let token = auth.issue_session().unwrap().access_token;
  let principal = auth.authorize(&headers(&token)).unwrap();
  auth.shutdown();
  assert!(principal.cancellation.is_cancelled());
  assert!(auth.authorize(&headers(&token)).is_err());
  assert!(auth.issue_session().is_err());
}

#[tokio::test]
async fn anonymous_login_capacity_preserves_new_logins_and_owner_registration() {
  let (_directory, auth) = setup();
  let secret = auth.bootstrap_token().unwrap().unwrap();
  let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));
  let request = registration(&auth, &mut authenticator, None, Some(&secret)).await;
  let (status, response) = post_json(
    auth.router(),
    "/hub/v1/auth/register/finish",
    Some(ORIGIN),
    None,
    request,
  )
  .await;
  assert_eq!(status, StatusCode::OK);
  let token = response["access_token"].as_str().unwrap();

  let oldest = login(&auth, &mut authenticator).await;
  for _ in 1..MAX_AUTHENTICATION_CEREMONIES {
    assert_eq!(
      post_json(auth.router(), "/hub/v1/auth/login/start", Some(ORIGIN), None, json!({}))
        .await
        .0,
      StatusCode::OK
    );
  }
  assert_eq!(auth.memory().unwrap().ceremonies.len(), MAX_AUTHENTICATION_CEREMONIES);
  let newest = login(&auth, &mut authenticator).await;
  assert_eq!(auth.memory().unwrap().ceremonies.len(), MAX_AUTHENTICATION_CEREMONIES);
  assert_eq!(
    post_json(auth.router(), "/hub/v1/auth/login/finish", Some(ORIGIN), None, oldest)
      .await
      .0,
    StatusCode::BAD_REQUEST
  );

  let (status, registration) = post_json(
    auth.router(),
    "/hub/v1/auth/register/start",
    Some(ORIGIN),
    Some(token),
    json!({}),
  )
  .await;
  assert_eq!(status, StatusCode::OK, "{registration}");
  // Anonymous eviction cannot remove the owner's registration ceremony either.
  for _ in 0..MAX_AUTHENTICATION_CEREMONIES {
    assert_eq!(
      post_json(auth.router(), "/hub/v1/auth/login/start", Some(ORIGIN), None, json!({}))
        .await
        .0,
      StatusCode::OK
    );
  }
  assert!(
    auth
      .memory()
      .unwrap()
      .ceremonies
      .contains_key(registration["ceremony_id"].as_str().unwrap())
  );
  assert_eq!(
    auth.memory().unwrap().ceremonies.len(),
    MAX_AUTHENTICATION_CEREMONIES + 1
  );
  // The earlier newest attempt was evicted by the second batch; a newly started
  // real login still completes successfully after saturation.
  assert_eq!(
    post_json(auth.router(), "/hub/v1/auth/login/finish", Some(ORIGIN), None, newest)
      .await
      .0,
    StatusCode::BAD_REQUEST
  );
  let fresh = login(&auth, &mut authenticator).await;
  assert_eq!(
    post_json(auth.router(), "/hub/v1/auth/login/finish", Some(ORIGIN), None, fresh)
      .await
      .0,
    StatusCode::OK
  );
}

#[test]
fn only_https_domain_origins_or_localhost_are_accepted() {
  assert!(validate_public_url("https://hub.example.com").is_ok());
  assert!(validate_public_url("http://localhost:5560").is_ok());
  for invalid in [
    "http://hub.example.com",
    "https://127.0.0.1",
    "https://[::1]",
    "https://hub.example.com/app",
    "https://hub.example.com?x=1",
    "https://u:p@hub.example.com",
    "https://hub.example.com#secret",
  ] {
    assert!(validate_public_url(invalid).is_err(), "{invalid}");
  }
}
