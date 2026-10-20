//! The way back to mayorana.ch from inside the app.
//!
//! People get PolishCast as a download and may never see the site again: the
//! update banner only shows when there is an update. The Feedback button is
//! always in the sidebar, so anyone with an idea or a problem has somewhere to
//! send it.

/// The contact form with the topic set and the app and version filled in, so a
/// message says what it is about without the sender having to.
pub fn contact_url() -> String {
    format!("https://mayorana.ch/en/contact?service=tools&app=PolishCast&v={}", env!("CARGO_PKG_VERSION"))
}
