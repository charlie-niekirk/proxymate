//! Secrets as generic passwords in the login keychain, service
//! [`crate::secrets::SERVICE`], one account per key.

use security_framework::passwords;

use crate::PlatformError;
use crate::secrets::{SERVICE, SecretStore};

const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

pub struct KeychainSecrets;

impl SecretStore for KeychainSecrets {
    fn get(&self, key: &str) -> Result<Option<String>, PlatformError> {
        match passwords::get_generic_password(SERVICE, key) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| PlatformError::Os(format!("the keychain item {key:?} isn't text"))),
            Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(err) => Err(PlatformError::Os(err.to_string())),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), PlatformError> {
        passwords::set_generic_password(SERVICE, key, secret.as_bytes()).map_err(|e| PlatformError::Os(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), PlatformError> {
        match passwords::delete_generic_password(SERVICE, key) {
            Err(err) if err.code() != ERR_SEC_ITEM_NOT_FOUND => Err(PlatformError::Os(err.to_string())),
            _ => Ok(()),
        }
    }
}
