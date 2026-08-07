use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::{AppSettingsRecord, StateStore, StoreError, CURRENT_SCHEMA_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateAppSettingsInput {
    pub theme: Theme,
}

#[derive(Debug, Serialize)]
pub struct AppSettings {
    pub locale: String,
    pub theme: String,
    pub launch_at_login_desired: bool,
    pub close_to_tray_notice_seen: bool,
    pub onboarding_completed: bool,
    pub last_update_check_at: Option<String>,
}

impl From<AppSettingsRecord> for AppSettings {
    fn from(record: AppSettingsRecord) -> Self {
        Self {
            locale: record.locale,
            theme: record.theme,
            launch_at_login_desired: record.launch_at_login_desired,
            close_to_tray_notice_seen: record.close_to_tray_notice_seen,
            onboarding_completed: record.onboarding_completed,
            last_update_check_at: record.last_update_check_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BootstrapState {
    pub schema_version: u32,
    pub settings: AppSettings,
}

#[derive(Debug, Serialize)]
pub struct PublicStoreError {
    pub code: &'static str,
}

impl From<StoreError> for PublicStoreError {
    fn from(_: StoreError) -> Self {
        Self {
            code: "state_unavailable",
        }
    }
}

fn bootstrap_from(record: AppSettingsRecord) -> BootstrapState {
    BootstrapState {
        schema_version: CURRENT_SCHEMA_VERSION,
        settings: record.into(),
    }
}

#[tauri::command]
pub fn update_app_settings(
    input: UpdateAppSettingsInput,
    store: State<'_, StateStore>,
) -> Result<BootstrapState, PublicStoreError> {
    store
        .update_theme(input.theme.as_str())
        .map(bootstrap_from)
        .map_err(Into::into)
}

#[tauri::command]
pub fn bootstrap_state(store: State<'_, StateStore>) -> Result<BootstrapState, PublicStoreError> {
    store.settings().map(bootstrap_from).map_err(Into::into)
}
