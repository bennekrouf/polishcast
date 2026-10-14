//! Polishcast Pro licences, checked offline, and the free version's ten exports.
//!
//! mayorana.ch signs each licence when it is bought with a private Ed25519 key; the app holds
//! the public half and checks the signature, so no account, network or activation is
//! involved. The key a buyer pastes is
//!
//! ```text
//! <base64url(payload JSON)>.<base64url(signature of those bytes)>
//! ```
//!
//! with the payload `{"v":1,"id","product","edition","email","issued","updates_until"}` — the
//! same format as TrackCut's and GitAgent's. A licence unlocks every release dated up to
//! `updates_until`, and keeps unlocking those releases after that day.
//!
//! Without Pro, [`FREE_EXPORTS`] different takes can be exported. Exporting one of them again
//! (after changing a zoom, say) doesn't count again; recording and editing are never limited.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

/// The product id in every Polishcast licence. The app was called Small Video when the first
/// keys were sold, and a key names its product, so the id stays.
pub const PRODUCT: &str = "small-video";

/// How many different takes the free version exports.
pub const FREE_EXPORTS: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct License {
    pub id: String,
    pub product: String,
    pub edition: String,
    pub email: String,
    /// `YYYY-MM-DD`.
    pub issued: String,
    /// `YYYY-MM-DD`: the last release date this licence unlocks.
    pub updates_until: String,
}

impl License {
    /// Whether this licence unlocks a release made on `release_date` (`YYYY-MM-DD`). ISO dates
    /// compare correctly as strings.
    pub fn covers(&self, release_date: &str) -> bool {
        release_date <= self.updates_until.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LicenseError {
    /// Not something a licence key looks like (truncated, or not a key at all).
    Malformed,
    /// Well-formed, but not signed by mayorana.ch: edited, or made up.
    BadSignature,
    /// A genuine licence, for another app.
    OtherProduct(String),
}

impl std::fmt::Display for LicenseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "This isn't a complete licence key. Copy the whole key from the email."),
            Self::BadSignature => write!(f, "This licence key isn't valid."),
            Self::OtherProduct(p) => write!(f, "This is a licence for {p}, not Polishcast."),
        }
    }
}

/// The licence in `key`, if `public_key` signed it and it is for Polishcast. Whitespace is
/// ignored, since keys pasted from an email are often wrapped.
pub fn verify(key: &str, public_key: &[u8; 32]) -> Result<License, LicenseError> {
    let key: String = key.chars().filter(|c| !c.is_whitespace()).collect();
    let (body, signature) = key.split_once('.').ok_or(LicenseError::Malformed)?;
    let payload = URL_SAFE_NO_PAD.decode(body).map_err(|_| LicenseError::Malformed)?;
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| LicenseError::Malformed)?;
    let signature = Signature::from_slice(&signature).map_err(|_| LicenseError::Malformed)?;
    let verifying = VerifyingKey::from_bytes(public_key).map_err(|_| LicenseError::BadSignature)?;
    verifying.verify_strict(&payload, &signature).map_err(|_| LicenseError::BadSignature)?;
    let license: License = serde_json::from_slice(&payload).map_err(|_| LicenseError::Malformed)?;
    if license.product != PRODUCT {
        return Err(LicenseError::OtherProduct(license.product));
    }
    Ok(license)
}

/// The takes exported so far without Pro, by name (a take's folder name), in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exported {
    #[serde(default)]
    pub takes: Vec<String>,
}

impl Exported {
    /// Whether `take` may be exported without Pro: it already was, or a free export is left.
    pub fn allows(&self, take: &str) -> bool {
        self.takes.iter().any(|t| t == take) || self.takes.len() < FREE_EXPORTS
    }

    /// Free exports not used yet.
    pub fn left(&self) -> usize {
        FREE_EXPORTS.saturating_sub(self.takes.len())
    }

    /// Counts a finished export of `take`; again for the same take changes nothing. Returns
    /// whether anything changed.
    pub fn record(&mut self, take: &str) -> bool {
        if self.takes.iter().any(|t| t == take) {
            return false;
        }
        self.takes.push(take.to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn sign(json: &str, key: &SigningKey) -> String {
        format!("{}.{}", URL_SAFE_NO_PAD.encode(json), URL_SAFE_NO_PAD.encode(key.sign(json.as_bytes()).to_bytes()))
    }

    const PAYLOAD: &str = r#"{"v":1,"id":"lic_1","product":"small-video","edition":"pro","email":"anna@studio.ch","issued":"2026-10-09","updates_until":"2027-10-09"}"#;

    #[test]
    fn a_signed_key_is_accepted_even_when_wrapped_by_an_email() {
        let signer = SigningKey::from_bytes(&[7u8; 32]);
        let key = sign(PAYLOAD, &signer);
        let (a, b) = key.split_at(40);
        let license = verify(&format!(" {a}\n{b}\n"), &signer.verifying_key().to_bytes()).unwrap();
        assert_eq!(license.email, "anna@studio.ch");
        assert!(license.covers("2027-10-09"));
        assert!(!license.covers("2027-10-10"));
    }

    #[test]
    fn forged_edited_junk_and_other_product_keys_are_refused() {
        let signer = SigningKey::from_bytes(&[7u8; 32]);
        let public = signer.verifying_key().to_bytes();
        let forger = SigningKey::from_bytes(&[8u8; 32]);
        assert_eq!(verify(&sign(PAYLOAD, &forger), &public), Err(LicenseError::BadSignature));
        let key = sign(PAYLOAD, &signer);
        let (_, sig) = key.split_once('.').unwrap();
        let edited = URL_SAFE_NO_PAD.encode(PAYLOAD.replace("2027", "2099"));
        assert_eq!(verify(&format!("{edited}.{sig}"), &public), Err(LicenseError::BadSignature));
        assert_eq!(verify("hello", &public), Err(LicenseError::Malformed));
        let other = sign(&PAYLOAD.replace("small-video", "splitter"), &signer);
        assert_eq!(verify(&other, &public), Err(LicenseError::OtherProduct("splitter".into())));
    }

    #[test]
    fn ten_different_takes_export_free_and_exporting_one_again_costs_nothing() {
        let mut exported = Exported::default();
        for i in 0..FREE_EXPORTS {
            let take = format!("2026-10-0{} 10.00", i);
            assert!(exported.allows(&take));
            assert!(exported.record(&take));
        }
        assert_eq!(exported.left(), 0);
        assert!(exported.allows("2026-10-03 10.00"), "a take already exported can be exported again");
        assert!(!exported.record("2026-10-03 10.00"));
        assert!(!exported.allows("2026-10-09 18.30"), "an eleventh take needs Pro");
    }
}
