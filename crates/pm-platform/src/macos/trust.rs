//! CA trust in the login keychain: `SecTrustSettingsSetTrustSettings` in the
//! user domain, with `security add-trusted-cert` as the fallback.

use std::process::Command;

use security_framework::base::Error as SecError;
use security_framework::certificate::SecCertificate;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
use security_framework::trust_settings::{Domain, TrustSettings, TrustSettingsForCertificate};

use crate::PlatformError;
use crate::trust::{CaCert, TrustState, TrustStore};

const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
const ERR_SEC_AUTH_FAILED: i32 = -25293;
const ERR_SEC_USER_CANCELED: i32 = -128;
const ERR_AUTHORIZATION_CANCELED: i32 = -60006;

pub struct Keychain;

impl TrustStore for Keychain {
    fn state(&self, ca: CaCert<'_>) -> Result<TrustState, PlatformError> {
        let cert = certificate(ca.der)?;
        let mut state = TrustState::NotTrusted;
        for (domain, raw) in [(Domain::User, 0), (Domain::Admin, 1)] {
            if has_empty_trust_settings(&cert, raw) {
                return Ok(TrustState::Trusted);
            }
            match TrustSettings::new(domain).tls_trust_settings_for_certificate(&cert) {
                Ok(Some(TrustSettingsForCertificate::TrustRoot | TrustSettingsForCertificate::TrustAsRoot)) => {
                    return Ok(TrustState::Trusted);
                }
                Ok(Some(TrustSettingsForCertificate::Deny)) => state = TrustState::Denied,
                Ok(_) => {}
                Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => {}
                Err(err) => return Err(os_error(err)),
            }
        }
        Ok(state)
    }

    fn install(&self, ca: CaCert<'_>) -> Result<(), PlatformError> {
        let cert = certificate(ca.der)?;
        match cert.add_to_keychain(None) {
            Ok(()) => {}
            Err(err) if err.code() == ERR_SEC_DUPLICATE_ITEM => {}
            Err(err) => return Err(os_error(err)),
        }
        match TrustSettings::new(Domain::User).set_trust_settings_always(&cert) {
            Ok(()) => Ok(()),
            Err(err) if is_cancel(err) => Err(PlatformError::Cancelled),
            // e.g. errSecInternalComponent outside a GUI session.
            Err(_) => security(&[
                "add-trusted-cert",
                "-r",
                "trustRoot",
                "-k",
                &login_keychain(),
                &ca.pem_path.to_string_lossy(),
            ]),
        }
    }

    fn uninstall(&self, ca: CaCert<'_>) -> Result<(), PlatformError> {
        let cert = certificate(ca.der)?;
        match remove_trust_settings(&cert) {
            Ok(()) => {}
            Err(code) if code == ERR_SEC_ITEM_NOT_FOUND => {}
            Err(code) if is_cancel(SecError::from_code(code)) => return Err(PlatformError::Cancelled),
            Err(_) => security(&["remove-trusted-cert", &ca.pem_path.to_string_lossy()])?,
        }
        delete_from_keychains(ca)
    }
}

fn certificate(der: &[u8]) -> Result<SecCertificate, PlatformError> {
    SecCertificate::from_der(der).map_err(os_error)
}

/// Deletes every keychain copy of the certificate.
fn delete_from_keychains(ca: CaCert<'_>) -> Result<(), PlatformError> {
    let found = ItemSearchOptions::new()
        .class(ItemClass::certificate())
        .load_refs(true)
        .limit(Limit::All)
        .search();
    let results = match found {
        Ok(results) => results,
        Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => return Ok(()),
        Err(err) => return Err(os_error(err)),
    };
    let mut failed = false;
    for result in results {
        if let SearchResult::Ref(Reference::Certificate(cert)) = result
            && cert.to_der() == ca.der
            && cert.delete().is_err()
        {
            failed = true;
        }
    }
    if failed {
        security(&["delete-certificate", "-Z", ca.sha1_hex, &login_keychain()])?;
    }
    Ok(())
}

/// An empty trust settings array means "always trust" (what
/// `set_trust_settings_always` writes), but security-framework's
/// `tls_trust_settings_for_certificate` reports it as no setting at all.
#[allow(unsafe_code)] // The raw call, to see the array's length.
fn has_empty_trust_settings(cert: &SecCertificate, domain: u32) -> bool {
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType as _};
    use security_framework_sys::trust_settings::SecTrustSettingsCopyTrustSettings;

    let mut array = std::ptr::null();
    // SAFETY: `cert` is live for the call and `array` is an out pointer; on
    // success we own the returned array (create rule).
    let status = unsafe { SecTrustSettingsCopyTrustSettings(cert.as_concrete_TypeRef(), domain, &mut array) };
    if status != 0 || array.is_null() {
        return false;
    }
    // SAFETY: a non-null CFArrayRef returned under the create rule.
    let array = unsafe { CFArray::<CFType>::wrap_under_create_rule(array) };
    array.is_empty()
}

#[allow(unsafe_code)] // security-framework has no wrapper for this call.
fn remove_trust_settings(cert: &SecCertificate) -> Result<(), i32> {
    use core_foundation::base::TCFType as _;
    use security_framework_sys::base::SecCertificateRef;

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecTrustSettingsRemoveTrustSettings(cert: SecCertificateRef, domain: u32) -> i32;
    }
    const USER_DOMAIN: u32 = 0;
    // SAFETY: `cert` is a live SecCertificateRef for the whole call; the
    // function only reads it.
    let status = unsafe { SecTrustSettingsRemoveTrustSettings(cert.as_concrete_TypeRef(), USER_DOMAIN) };
    if status == 0 { Ok(()) } else { Err(status) }
}

fn is_cancel(err: SecError) -> bool {
    matches!(
        err.code(),
        ERR_SEC_AUTH_FAILED | ERR_SEC_USER_CANCELED | ERR_AUTHORIZATION_CANCELED
    )
}

fn os_error(err: SecError) -> PlatformError {
    PlatformError::Os(
        err.message()
            .unwrap_or_else(|| format!("Security framework error {}", err.code())),
    )
}

fn login_keychain() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Library/Keychains/login.keychain-db")
}

fn security(args: &[&str]) -> Result<(), PlatformError> {
    let out = Command::new("/usr/bin/security").args(args).output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if stderr.contains("canceled") || stderr.contains("cancelled") {
        return Err(PlatformError::Cancelled);
    }
    Err(PlatformError::Os(format!("security {}: {stderr}", args[0])))
}
