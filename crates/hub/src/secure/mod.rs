//! Endpoint-owned identities, owner-signed grants, and authenticated Noise records.
//!
//! Provision the owner's verification key and the host's Noise public key over
//! a trusted channel. The Hub must never choose either trust anchor. An
//! authenticated Noise peer is not yet authorized: hosts must verify its grant
//! and enforce the grant's scope before accessing the local viewer API.
mod grant;
mod identity;
mod noise;

pub use grant::{GRANT_VERSION, Grant, GrantScope, MAX_GRANT_BYTES, SignedGrant};
pub use identity::{NoiseIdentity, OwnerIdentity, decode_public_key};
pub use noise::{
  InnerMessage, MAX_CHUNK, MAX_PLAINTEXT, MAX_RECORD, MAX_REQUEST_BODY, NoiseInitiator, NoiseResponder, SecureChannel,
};
