use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};

use super::*;

pub(crate) async fn post_json(
  app: Router,
  path: &str,
  origin: Option<&str>,
  token: Option<&str>,
  body: Value,
) -> (StatusCode, Value) {
  let mut builder = Request::builder()
    .method("POST")
    .uri(path)
    .header(header::CONTENT_TYPE, "application/json");
  if let Some(origin) = origin {
    builder = builder.header(header::ORIGIN, origin);
  }
  if let Some(token) = token {
    builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
  }
  let response = app
    .oneshot(builder.body(Body::from(body.to_string())).unwrap())
    .await
    .unwrap();
  let status = response.status();
  let bytes = response.into_body().collect().await.unwrap().to_bytes();
  let value = if bytes.is_empty() {
    Value::Null
  } else {
    serde_json::from_slice(&bytes).unwrap()
  };
  (status, value)
}

pub(crate) async fn login_for_test(app: Router, auth: &AuthState) -> String {
  let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));
  let bootstrap_token = auth.bootstrap_token().unwrap().expect("fresh test installation");
  let (status, start) = post_json(
    app.clone(),
    "/hub/v1/auth/register/start",
    Some(&auth.inner.origin),
    None,
    json!({"bootstrap_token": bootstrap_token}),
  )
  .await;
  assert_eq!(status, StatusCode::OK, "{start}");
  let credential = authenticator
    .do_registration(
      Url::parse(&auth.inner.origin).unwrap(),
      serde_json::from_value(start["options"].clone()).unwrap(),
    )
    .unwrap();
  let (status, finish) = post_json(
    app,
    "/hub/v1/auth/register/finish",
    Some(&auth.inner.origin),
    None,
    json!({"ceremony_id": start["ceremony_id"], "credential": credential}),
  )
  .await;
  assert_eq!(status, StatusCode::OK, "{finish}");
  finish["access_token"].as_str().unwrap().to_owned()
}
