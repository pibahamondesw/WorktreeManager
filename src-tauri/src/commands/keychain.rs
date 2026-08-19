//! Secret storage backed by the macOS login keychain.
//!
//! Everything lives in a single item rather than one per workspace: the keychain ACL is
//! per item, so N items would mean N authorization prompts the first time a new build
//! reads them. Callers treat the value as an opaque string; the shape of what is stored
//! is the frontend's concern.

use keyring::{Entry, Error};

/// Keychain attributes identifying our item. `SERVICE` is what shows up in Keychain
/// Access; it is only a label, unrelated to the code signature the ACL is keyed on.
const SERVICE: &str = "com.worktreemanager.dev";
const ACCOUNT: &str = "linear-api-keys";

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| format!("Failed to open keychain entry: {e}"))
}

/// Read the stored secret, or `None` when nothing has been stored yet.
///
/// The empty case is deliberately not an error: a denied authorization prompt fails
/// instead, so a caller about to drop its own on-disk copy can tell "nothing here" from
/// "could not look" and keep the copy in the second case.
#[tauri::command]
pub fn keychain_get() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Failed to read from keychain: {e}")),
    }
}

#[tauri::command]
pub fn keychain_set(value: String) -> Result<(), String> {
    entry()?
        .set_password(&value)
        .map_err(|e| format!("Failed to write to keychain: {e}"))
}
