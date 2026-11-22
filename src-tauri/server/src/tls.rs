//! The external listener's certificate: generated into `tls/` with rcgen and pinned by fingerprint,
//! or the user's own. The generated one is kept across restarts and regenerated only when its SAN
//! set (the advertise host plus the bind IP when specific) changes, which makes clients re-pair.

use std::net::IpAddr;
use std::path::Path;

pub struct Identity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: String,
}

pub struct IdentityParams<'a> {
    /// The `tls/` folder.
    pub dir: &'a Path,
    pub advertise: &'a str,
    /// The external bind address; it joins the SAN set unless it is a wildcard.
    pub bind: Option<IpAddr>,
}

/// The kept certificate when its SAN set matches, else a new one written to `dir`.
pub fn self_signed(params: IdentityParams) -> anyhow::Result<Identity> {
    let _ = params;
    todo!("tls::self_signed")
}

/// Lowercase hex SHA-256 of the certificate's DER bytes, as pairing strings carry it.
pub fn fingerprint(cert_der: &[u8]) -> String {
    let _ = cert_der;
    todo!("tls::fingerprint")
}

/// Installs ring as rustls's process-wide provider (aws-lc-rs is never built).
pub fn install_provider() {
    todo!("tls::install_provider")
}
