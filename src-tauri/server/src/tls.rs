//! The external listener's certificate: generated into `tls/` with rcgen and pinned by fingerprint,
//! or the user's own. The generated one is kept across restarts and regenerated only when its SAN
//! set (the advertise host plus the bind IP when specific) changes, which makes clients re-pair.

use std::net::IpAddr;
use std::path::Path;

use anyhow::Context;

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
    let mut sans = vec![params.advertise.to_string()];
    if let Some(ip) = params.bind.filter(|ip| !ip.is_unspecified()) {
        let ip = ip.to_string();
        if !sans.contains(&ip) {
            sans.push(ip);
        }
    }
    let (cert_file, key_file, sans_file) = (params.dir.join("cert.der"), params.dir.join("key.der"), params.dir.join("sans.json"));
    let kept_sans: Option<Vec<String>> = std::fs::read(&sans_file).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if kept_sans.as_ref() == Some(&sans)
        && let (Ok(cert_der), Ok(key_der)) = (std::fs::read(&cert_file), std::fs::read(&key_file))
    {
        let fingerprint = fingerprint(&cert_der);
        return Ok(Identity { cert_der, key_der, fingerprint });
    }
    let generated = rcgen::generate_simple_self_signed(sans.clone()).context("Couldn't generate a TLS certificate")?;
    let cert_der = generated.cert.der().to_vec();
    let key_der = generated.signing_key.serialize_der();
    std::fs::create_dir_all(params.dir).with_context(|| format!("Couldn't create {}", params.dir.display()))?;
    write_private(&key_file, &key_der)?;
    write_private(&cert_file, &cert_der)?;
    // Written last: a crash part way leaves no SAN record, so the next start generates again.
    write_private(&sans_file, &serde_json::to_vec(&sans)?)?;
    let fingerprint = fingerprint(&cert_der);
    Ok(Identity { cert_der, key_der, fingerprint })
}

fn write_private(file: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(file).and_then(|mut f| f.write_all(bytes)).with_context(|| format!("Couldn't write {}", file.display()))
}

/// The user's own certificate chain and key, from PEM files.
pub fn own_cert(cert: &Path, key: &Path) -> anyhow::Result<(Vec<Vec<u8>>, Vec<u8>, String)> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let chain: Vec<Vec<u8>> = CertificateDer::pem_file_iter(cert)
        .and_then(|certs| certs.map(|c| c.map(|c| c.to_vec())).collect())
        .with_context(|| format!("Couldn't read the certificate {}", cert.display()))?;
    let first = chain.first().with_context(|| format!("{} holds no certificate", cert.display()))?;
    let fingerprint = fingerprint(first);
    let key = PrivateKeyDer::from_pem_file(key).with_context(|| format!("Couldn't read the key {}", key.display()))?;
    Ok((chain, key.secret_der().to_vec(), fingerprint))
}

/// A rustls server config on the ring provider, offering HTTP/2 and HTTP/1.1.
pub fn server_config(chain: Vec<Vec<u8>>, key_der: Vec<u8>) -> anyhow::Result<rustls::ServerConfig> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let chain = chain.into_iter().map(CertificateDer::from).collect();
    let key = PrivateKeyDer::try_from(key_der).map_err(|e| anyhow::anyhow!("Couldn't read the TLS key: {e}"))?;
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .context("Couldn't use the TLS certificate")?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// Lowercase hex SHA-256 of the certificate's DER bytes, as pairing strings carry it.
pub fn fingerprint(cert_der: &[u8]) -> String {
    crate::auth::hex(ring::digest::digest(&ring::digest::SHA256, cert_der).as_ref())
}

/// Installs ring as rustls's process-wide provider (aws-lc-rs is never built).
pub fn install_provider() {
    // Fails only when a provider is already installed, which is what we want either way.
    let _ = rustls::crypto::ring::default_provider().install_default();
}
