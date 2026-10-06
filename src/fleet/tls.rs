//! Mutual-TLS for the agent↔hub link.
//!
//! The hub is a tiny private CA. `hub init` generates the CA and the hub's server
//! certificate; `hub enroll <node>` mints a client certificate for an agent,
//! signed by the CA, and records its fingerprint in an enrolled allowlist.
//!
//! On the wire, the hub requires a client certificate that (a) chains to its CA
//! and (b) whose SHA-256 fingerprint is in `enrolled.txt` — so a single agent can
//! be revoked by deleting one line. The agent verifies the hub's certificate
//! against the same CA.

use std::collections::HashSet;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

use directories::ProjectDirs;
use rcgen::{BasicConstraints, CertificateParams, DnType, Ia5String, IsCa, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use sha2::{Digest, Sha256};

/// Install the ring crypto provider once (rustls has no default without it when
/// built with `default-features = false`).
pub fn install_crypto() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// `~/.config/porticus/hub` — where the CA, server cert, and enrolled list live.
pub fn hub_dir() -> Result<PathBuf, String> {
    ProjectDirs::from("com", "porticus", "porticus")
        .map(|p| p.config_dir().join("hub"))
        .ok_or_else(|| "cannot locate config directory".to_string())
}

/// SHA-256 fingerprint of a DER certificate, lowercase hex.
pub fn fingerprint_hex(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn san_for(value: &str) -> Result<SanType, String> {
    if let Ok(ip) = value.parse::<IpAddr>() {
        Ok(SanType::IpAddress(ip))
    } else {
        Ia5String::try_from(value.to_string())
            .map(SanType::DnsName)
            .map_err(|e| format!("invalid SAN {value:?}: {e}"))
    }
}

/// Generate the CA and the hub server certificate. `sans` are the addresses
/// agents will dial the hub at (defaults to localhost/127.0.0.1).
pub fn init(mut sans: Vec<String>) -> Result<PathBuf, String> {
    install_crypto();
    let dir = hub_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

    // CA.
    let ca_key = KeyPair::generate().map_err(|e| format!("ca key: {e}"))?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "porticus-hub-ca");
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| format!("ca cert: {e}"))?;
    write_private(&dir.join("ca.key"), &ca_key.serialize_pem())?;
    fs::write(dir.join("ca.crt"), ca_cert.pem()).map_err(|e| e.to_string())?;

    // Hub server cert, signed by the CA.
    if sans.is_empty() {
        sans = vec!["localhost".into(), "127.0.0.1".into()];
    }
    let srv_key = KeyPair::generate().map_err(|e| format!("server key: {e}"))?;
    let mut srv_params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    srv_params
        .distinguished_name
        .push(DnType::CommonName, "porticus-hub");
    for s in &sans {
        srv_params.subject_alt_names.push(san_for(s)?);
    }
    let srv_cert = srv_params
        .signed_by(&srv_key, &ca_cert, &ca_key)
        .map_err(|e| format!("server cert: {e}"))?;
    write_private(&dir.join("server.key"), &srv_key.serialize_pem())?;
    fs::write(dir.join("server.crt"), srv_cert.pem()).map_err(|e| e.to_string())?;

    Ok(dir)
}

/// Mint a client certificate for `node`, signed by the CA, writing
/// `<node>.crt`, `<node>.key`, and a copy of `ca.crt` into `out`, and recording
/// the fingerprint in the enrolled allowlist. Returns the fingerprint.
pub fn enroll(node: &str, out: &Path) -> Result<String, String> {
    install_crypto();
    let dir = hub_dir()?;
    let ca_cert_pem = fs::read_to_string(dir.join("ca.crt"))
        .map_err(|e| format!("run `porticus hub init` first ({e})"))?;
    let ca_key_pem = fs::read_to_string(dir.join("ca.key")).map_err(|e| e.to_string())?;

    // Reconstruct the CA as a usable issuer (same DN + key → children verify
    // against the distributed ca.crt).
    let ca_key = KeyPair::from_pem(&ca_key_pem).map_err(|e| format!("ca key: {e}"))?;
    let ca_params =
        CertificateParams::from_ca_cert_pem(&ca_cert_pem).map_err(|e| format!("ca cert: {e}"))?;
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| format!("ca reissue: {e}"))?;

    let key = KeyPair::generate().map_err(|e| format!("agent key: {e}"))?;
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    params.distinguished_name.push(DnType::CommonName, node);
    let cert = params
        .signed_by(&key, &ca_cert, &ca_key)
        .map_err(|e| format!("agent cert: {e}"))?;

    fs::create_dir_all(out).map_err(|e| format!("create {}: {e}", out.display()))?;
    fs::write(out.join(format!("{node}.crt")), cert.pem()).map_err(|e| e.to_string())?;
    write_private(&out.join(format!("{node}.key")), &key.serialize_pem())?;
    fs::write(out.join("ca.crt"), &ca_cert_pem).map_err(|e| e.to_string())?;

    let fp = fingerprint_hex(cert.der());
    let line = format!("{fp} {node}\n");
    append(&dir.join("enrolled.txt"), &line)?;
    Ok(fp)
}

/// Fingerprints currently allowed to connect (one per enrolled, un-revoked cert).
pub fn load_enrolled() -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(dir) = hub_dir() {
        if let Ok(text) = fs::read_to_string(dir.join("enrolled.txt")) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(fp) = line.split_whitespace().next() {
                    set.insert(fp.to_string());
                }
            }
        }
    }
    set
}

/// Build the hub's mTLS server config from the files in the hub dir.
pub fn server_config() -> Result<Arc<ServerConfig>, String> {
    install_crypto();
    let dir = hub_dir()?;
    let certs = load_certs(&dir.join("server.crt"))?;
    let key = load_key(&dir.join("server.key"))?;
    let roots = ca_roots(&dir.join("ca.crt"))?;
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|e| format!("client verifier: {e}"))?;
    let cfg = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|e| format!("server config: {e}"))?;
    Ok(Arc::new(cfg))
}

/// Build the agent's mTLS client config: present our cert, verify the hub against
/// the CA.
pub fn client_config(ca: &str, cert: &str, key: &str) -> Result<Arc<ClientConfig>, String> {
    install_crypto();
    let roots = ca_roots(Path::new(ca))?;
    let certs = load_certs(Path::new(cert))?;
    let key = load_key(Path::new(key))?;
    let cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(certs, key)
        .map_err(|e| format!("client config: {e}"))?;
    Ok(Arc::new(cfg))
}

fn ca_roots(path: &Path) -> Result<RootCertStore, String> {
    let mut roots = RootCertStore::empty();
    for cert in load_certs(path)? {
        roots.add(cert).map_err(|e| format!("add ca: {e}"))?;
    }
    Ok(roots)
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let data = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    rustls_pemfile::certs(&mut &data[..])
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse {}: {e}", path.display()))
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>, String> {
    let data = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    rustls_pemfile::private_key(&mut &data[..])
        .map_err(|e| format!("parse {}: {e}", path.display()))?
        .ok_or_else(|| format!("no private key in {}", path.display()))
}

fn append(path: &Path, line: &str) -> Result<(), String> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    f.write_all(line.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))
}

/// Write a file containing a private key with owner-only permissions on unix.
fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|e| format!("write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_hex() {
        let fp = fingerprint_hex(b"hello");
        assert_eq!(fp.len(), 64);
        assert_eq!(
            fp,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
