//! The Proxymate root CA and the leaf certificates it issues for
//! intercepted hosts.
//!
//! The CA is ECDSA P-256 and lives in `<data_dir>/ca/`, the key mode 0600.
//! Every leaf shares one P-256 keypair made at startup, so issuing a leaf is
//! one signature (tens of µs); leaves are cached per host.

use std::fmt;
use std::io;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moka::sync::Cache;
use pm_model::CaInfo;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType, SerialNumber,
};
use rustls::crypto::ring::sign::any_ecdsa_type;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::{CertifiedKey, SigningKey};
use time::OffsetDateTime;

use super::cert_summary::{rfc3339, sha256_hex};

const CERT_FILE: &str = "proxymate-ca.pem";
const KEY_FILE: &str = "proxymate-ca-key.pem";
const CA_VALIDITY: time::Duration = time::Duration::days(3650);
/// Leaves are backdated a day for clock skew and valid 365 days in total.
const LEAF_BACKDATE: time::Duration = time::Duration::days(1);
const LEAF_VALIDITY: time::Duration = time::Duration::days(364);
const LEAF_CACHE_ENTRIES: u64 = 2_000;
const LEAF_CACHE_TTL: Duration = Duration::from_secs(12 * 3600);

#[derive(Debug, thiserror::Error)]
pub enum CaError {
    #[error("certificate generation: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("CA files: {0}")]
    Io(#[from] io::Error),
    #[error("leaf key: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("{0:?} is not a valid host name for a certificate")]
    BadHost(String),
}

pub struct CertAuthority {
    issuer: Issuer<'static, KeyPair>,
    ca_der: CertificateDer<'static>,
    not_after: OffsetDateTime,
    info: CaInfo,
    leaf_key: KeyPair,
    leaf_signer: Arc<dyn SigningKey>,
    leaves: Cache<String, Arc<CertifiedKey>>,
    issued: AtomicU64,
}

impl fmt::Debug for CertAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CertAuthority")
            .field("ca", &self.info.common_name)
            .finish()
    }
}

impl CertAuthority {
    /// Loads the CA from `dir`, or creates and saves a new one if there is
    /// none, it is corrupt, or it is about to expire. An I/O error is
    /// returned rather than replacing a CA the user may already trust.
    pub fn load_or_create(dir: &Path) -> Result<CertAuthority, CaError> {
        match Self::load(dir) {
            Ok(Some(ca)) => Ok(ca),
            Err(CaError::Io(err)) => Err(CaError::Io(err)),
            Ok(None) | Err(_) => Self::create(dir),
        }
    }

    /// Makes a new CA, replacing any in `dir`. Clients that trusted the old
    /// one must trust the new one.
    pub fn create(dir: &Path) -> Result<CertAuthority, CaError> {
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, common_name(now));
        name.push(DnType::OrganizationName, "Proxymate");
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.not_before = now - LEAF_BACKDATE;
        params.not_after = now + CA_VALIDITY;
        params.serial_number = Some(random_serial());
        let cert = params.self_signed(&key)?;

        std::fs::create_dir_all(dir)?;
        pm_platform::fs::write_private(&dir.join(KEY_FILE), key.serialize_pem().as_bytes())?;
        write_atomic(&dir.join(CERT_FILE), cert.pem().as_bytes())?;
        Self::from_parts(dir, cert.der().clone(), key)
    }

    fn load(dir: &Path) -> Result<Option<CertAuthority>, CaError> {
        let (cert_path, key_path) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
        if !cert_path.exists() || !key_path.exists() {
            return Ok(None);
        }
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key = KeyPair::from_pem(&std::fs::read_to_string(&key_path)?)?;
        let der = rustls_pemfile_der(&cert_pem).ok_or(rcgen::Error::CouldNotParseCertificate)?;
        let ca = Self::from_parts(dir, der, key)?;
        // Re-create a CA close to expiry rather than issue leaves past it.
        Ok((ca.not_after - OffsetDateTime::now_utc() > LEAF_VALIDITY).then_some(ca))
    }

    fn from_parts(dir: &Path, ca_der: CertificateDer<'static>, key: KeyPair) -> Result<CertAuthority, CaError> {
        let (_, parsed) =
            x509_parser::parse_x509_certificate(&ca_der).map_err(|_| rcgen::Error::CouldNotParseCertificate)?;
        let validity = parsed.validity();
        let not_after = validity.not_after.to_datetime();
        let info = CaInfo {
            common_name: parsed
                .subject()
                .iter_common_name()
                .next()
                .and_then(|cn| cn.as_str().ok())
                .unwrap_or("Proxymate CA")
                .to_owned(),
            cert_path: dir.join(CERT_FILE).to_string_lossy().into_owned(),
            not_before: rfc3339(validity.not_before.to_datetime()),
            not_after: rfc3339(not_after),
            sha256: sha256_hex(&ca_der),
        };
        let issuer = Issuer::from_ca_cert_der(&ca_der, key)?;

        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
        let leaf_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        let leaf_signer = any_ecdsa_type(&leaf_der)?;
        Ok(CertAuthority {
            issuer,
            ca_der,
            not_after,
            info,
            leaf_key,
            leaf_signer,
            leaves: Cache::builder()
                .max_capacity(LEAF_CACHE_ENTRIES)
                .time_to_live(LEAF_CACHE_TTL)
                .build(),
            issued: AtomicU64::new(0),
        })
    }

    pub fn info(&self) -> &CaInfo {
        &self.info
    }

    pub fn cert_der(&self) -> &CertificateDer<'static> {
        &self.ca_der
    }

    /// Base64 SHA-256 of the CA's SubjectPublicKeyInfo, the form Chromium's
    /// `--ignore-certificate-errors-spki-list` takes (M21). Leaves carry the
    /// CA in their chain, so pinning it covers every leaf.
    pub fn spki_sha256_base64(&self) -> String {
        use base64::Engine as _;
        let spki = x509_parser::parse_x509_certificate(&self.ca_der)
            .map(|(_, cert)| cert.tbs_certificate.subject_pki.raw.to_vec())
            .unwrap_or_default();
        let digest = ring::digest::digest(&ring::digest::SHA256, &spki);
        base64::engine::general_purpose::STANDARD.encode(digest.as_ref())
    }

    /// The leaf for `host` (a DNS name or an IP literal), from the cache or
    /// freshly issued.
    pub fn leaf(&self, host: &str) -> Result<Arc<CertifiedKey>, Arc<CaError>> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.leaves.try_get_with_by_ref(&host, || self.issue(&host))
    }

    /// Leaves issued so far, i.e. cache misses.
    pub fn issued(&self) -> u64 {
        self.issued.load(Ordering::Relaxed)
    }

    fn issue(&self, host: &str) -> Result<Arc<CertifiedKey>, CaError> {
        let san = match host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
            Ok(ip) => SanType::IpAddress(ip),
            Err(_) => SanType::DnsName(host.try_into().map_err(|_| CaError::BadHost(host.to_owned()))?),
        };
        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, host);
        params.distinguished_name = name;
        params.subject_alt_names = vec![san];
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.not_before = now - LEAF_BACKDATE;
        params.not_after = (now + LEAF_VALIDITY).min(self.not_after);
        params.serial_number = Some(random_serial());
        params.use_authority_key_identifier_extension = true;
        let cert = params.signed_by(&self.leaf_key, &self.issuer)?;
        self.issued.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(CertifiedKey::new(
            vec![cert.der().clone(), self.ca_der.clone()],
            self.leaf_signer.clone(),
        )))
    }
}

/// "Proxymate CA (<machine>, <date>)": several machines' CAs can sit in
/// one keychain or one phone, and this tells them apart.
fn common_name(now: OffsetDateTime) -> String {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let host = host.trim_end_matches(".local");
    let date = now.date();
    format!(
        "Proxymate CA ({host}, {:04}-{:02}-{:02})",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

/// 16 random bytes, positive as DER requires. Leaves share a key, so the
/// serial is what makes each one unique.
fn random_serial() -> SerialNumber {
    let mut bytes = *uuid::Uuid::new_v4().as_bytes();
    bytes[0] &= 0x7f;
    bytes[0] |= 0x01;
    SerialNumber::from_slice(&bytes)
}

fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let tmp = pm_platform::fs::temp_sibling(path);
    std::fs::write(&tmp, contents)?;
    std::fs::rename(tmp, path)
}

/// The first certificate in a PEM file.
fn rustls_pemfile_der(pem: &str) -> Option<CertificateDer<'static>> {
    use rustls::pki_types::pem::PemObject;
    CertificateDer::from_pem_slice(pem.as_bytes()).ok()
}

/// Issues leaves by SNI. Built once into the shared server configs; a
/// client that sends no SNI (an IP literal) gets a resolver with a
/// `fallback` host instead.
#[derive(Debug)]
pub struct LeafResolver {
    pub ca: Arc<CertAuthority>,
    pub fallback: Option<String>,
}

impl ResolvesServerCert for LeafResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let host = hello.server_name().or(self.fallback.as_deref())?;
        self.ca.leaf(host).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pm-ca-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ca_is_created_once_and_reloaded() {
        let dir = temp_dir();
        let first = CertAuthority::load_or_create(&dir).unwrap();
        let again = CertAuthority::load_or_create(&dir).unwrap();
        assert_eq!(first.info(), again.info());
        assert!(first.info().common_name.starts_with("Proxymate CA ("));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(KEY_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let regenerated = CertAuthority::create(&dir).unwrap();
        assert_ne!(regenerated.info().sha256, first.info().sha256);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_spki_hash_is_of_the_ca_key() {
        let dir = temp_dir();
        let ca = CertAuthority::create(&dir).unwrap();
        let hash = ca.spki_sha256_base64();
        // 32 bytes of SHA-256 in padded Base64.
        assert_eq!(hash.len(), 44);
        // Every leaf's chain carries the CA, whose key it names.
        let leaf = ca.leaf("example.com").unwrap();
        assert_eq!(leaf.cert[1], *ca.cert_der());
        let again = CertAuthority::load_or_create(&dir).unwrap();
        assert_eq!(again.spki_sha256_base64(), hash);
        assert_ne!(CertAuthority::create(&temp_dir()).unwrap().spki_sha256_base64(), hash);
    }

    #[test]
    fn ca_has_the_ca_extensions() {
        let dir = temp_dir();
        let ca = CertAuthority::load_or_create(&dir).unwrap();
        let (_, cert) = x509_parser::parse_x509_certificate(ca.cert_der()).unwrap();
        assert!(cert.is_ca());
        let usage = cert.key_usage().unwrap().unwrap().value;
        assert!(usage.key_cert_sign() && usage.crl_sign());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn leaves_are_cached_per_host() {
        let dir = temp_dir();
        let ca = CertAuthority::load_or_create(&dir).unwrap();
        let a = ca.leaf("example.com").unwrap();
        let b = ca.leaf("Example.COM.").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(ca.issued(), 1);
        let other = ca.leaf("api.example.com").unwrap();
        assert!(!Arc::ptr_eq(&a, &other));
        assert_eq!(ca.issued(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn leaves_have_san_eku_validity_and_unique_serials() {
        let dir = temp_dir();
        let ca = CertAuthority::load_or_create(&dir).unwrap();
        let dns = ca.leaf("example.com").unwrap();
        let ip = ca.leaf("127.0.0.1").unwrap();
        let (_, dns_cert) = x509_parser::parse_x509_certificate(&dns.cert[0]).unwrap();
        let (_, ip_cert) = x509_parser::parse_x509_certificate(&ip.cert[0]).unwrap();

        let san = dns_cert.subject_alternative_name().unwrap().unwrap();
        assert!(matches!(
            san.value.general_names[0],
            x509_parser::extensions::GeneralName::DNSName("example.com")
        ));
        let san = ip_cert.subject_alternative_name().unwrap().unwrap();
        assert!(matches!(
            san.value.general_names[0],
            x509_parser::extensions::GeneralName::IPAddress(_)
        ));
        assert!(dns_cert.extended_key_usage().unwrap().unwrap().value.server_auth);
        assert!(!dns_cert.is_ca());

        let validity = dns_cert.validity();
        let span = validity.not_after.timestamp() - validity.not_before.timestamp();
        assert!(span <= 365 * 86_400, "valid for {span}s");
        assert!(validity.not_before.timestamp() < OffsetDateTime::now_utc().unix_timestamp() - 86_000);

        assert_ne!(dns_cert.raw_serial(), ip_cert.raw_serial());
        assert_eq!(
            dns_cert.issuer(),
            x509_parser::parse_x509_certificate(ca.cert_der()).unwrap().1.subject()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
