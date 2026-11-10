//! The CircleCI personal token CircleCI event triggers poll with. It lives in its own Keychain
//! item; the interface can save or remove it and ask whether one is saved, never read it back.

use std::sync::Arc;

use tauri::State;

/// One Keychain item.
pub trait SecretItem: Send + Sync {
    fn read(&self) -> Result<Option<String>, String>;
    /// Removes the item when `value` is `None`.
    fn write(&self, value: Option<&str>) -> Result<(), String>;
}

/// The Keychain item holding the CircleCI token.
#[derive(Clone)]
pub struct CircleciToken(pub Arc<dyn SecretItem>);

impl CircleciToken {
    pub fn read(&self) -> Result<Option<String>, String> {
        Ok(self.0.read()?.filter(|token| !token.trim().is_empty()))
    }

    fn saved(&self) -> Result<bool, String> {
        Ok(self.read()?.is_some())
    }

    /// A blank token removes the saved one.
    fn save(&self, token: Option<&str>) -> Result<(), String> {
        let token = token.map(str::trim).filter(|token| !token.is_empty());
        self.0.write(token)
    }
}

#[tauri::command]
pub fn circleci_token_saved(token: State<'_, CircleciToken>) -> Result<bool, String> {
    token.saved()
}

#[tauri::command]
pub fn circleci_token_save(
    token: State<'_, CircleciToken>,
    value: Option<String>,
) -> Result<(), String> {
    token.save(value.as_deref())
}
