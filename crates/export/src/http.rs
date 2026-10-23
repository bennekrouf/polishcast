//! The HTTPS client every PolishCast request starts from: the licence check with api0, the
//! update check and the ffmpeg download.
//!
//! Certificates are checked by the operating system (the Windows certificate store, the macOS
//! keychain, the Linux CA bundle) rather than against a list built into the app. A company
//! proxy that inspects connections (Zscaler and the like) re-signs them with its own root
//! certificate, which IT installs on the computer: trusted that way, PolishCast connects
//! wherever the browser does.

use ureq::config::ConfigBuilder;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::typestate::AgentScope;

/// What to do when the certificate isn't trusted, for the end of an error message.
pub const UNTRUSTED_HINT: &str = "This computer doesn't trust the certificate the connection came with. \
     On a company network, a proxy that inspects connections (such as Zscaler) is the usual cause: \
     ask IT to install its root certificate in this computer's certificate store, then try again.";

/// An agent's settings, with certificates checked by the operating system.
pub fn config() -> ConfigBuilder<AgentScope> {
    ureq::Agent::config_builder().tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
}

/// An agent with no other settings.
pub fn agent() -> ureq::Agent {
    config().build().into()
}

/// Whether `e` is a certificate the operating system didn't trust, typically a proxy's.
pub fn untrusted(e: &ureq::Error) -> bool {
    is_certificate_error(&e.to_string())
}

fn is_certificate_error(message: &str) -> bool {
    message.to_ascii_lowercase().contains("certificate")
}

/// `e`, followed by `UNTRUSTED_HINT` when that's the problem.
pub fn describe(e: &ureq::Error) -> String {
    if untrusted(e) {
        format!("{e}. {UNTRUSTED_HINT}")
    } else {
        e.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_certificate_errors() {
        // What rustls reports behind a proxy whose root isn't trusted.
        assert!(is_certificate_error("io: invalid peer certificate: UnknownIssuer"));
        assert!(!is_certificate_error("io: Connection refused (os error 61)"));
        assert!(!is_certificate_error("timeout: global"));
    }

    /// The agent builds with the platform verifier (it panics without the feature).
    #[test]
    fn agent_uses_the_platform_verifier() {
        let config = config().build();
        assert!(matches!(config.tls_config().root_certs(), RootCerts::PlatformVerifier));
        let _ = agent();
    }

    /// Needs the network: the OS accepts api0's certificate.
    /// `cargo test -p polishcast-export -- --ignored reaches_api0`
    #[test]
    #[ignore]
    fn reaches_api0() {
        let agent: ureq::Agent = config().http_status_as_error(false).build().into();
        let resp = agent.get("https://gateway.api0.ai/v1/licence").call().map_err(|e| describe(&e)).unwrap();
        assert_eq!(resp.status().as_u16(), 401);
    }
}
