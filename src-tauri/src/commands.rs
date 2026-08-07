use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    domain::{
        AppSettings as DomainAppSettings, EnvironmentId, EnvironmentKind, Locale,
        ManagedEnvironment, Provider, ProviderId, ProviderKind, ProviderVerification, SecretString,
        StateSnapshot, Theme as DomainTheme,
    },
    state::{AppSettingsRecord, StateStore, StoreError, CURRENT_SCHEMA_VERSION},
};

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

impl PublicStoreError {
    fn invalid_input() -> Self {
        Self {
            code: "invalid_state_input",
        }
    }
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

#[derive(Deserialize)]
pub struct ReplaceStateSnapshotInput {
    providers: Vec<ProviderInput>,
    verifications: Vec<ProviderVerificationInput>,
    environments: Vec<ManagedEnvironmentInput>,
    settings: CompleteAppSettingsInput,
}

#[derive(Deserialize)]
struct ProviderInput {
    id: String,
    provider_kind: String,
    built_in_key: Option<String>,
    display_name: String,
    base_url: Option<String>,
    api_key: Option<String>,
    default_model: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct ProviderVerificationInput {
    provider_id: String,
    combination_fingerprint: String,
    verified_at: String,
    contract_version: String,
}

#[derive(Deserialize)]
struct ManagedEnvironmentInput {
    id: String,
    environment_kind: String,
    platform_identity: String,
    display_name: String,
    current_provider_id: Option<String>,
    first_seen_at: String,
    last_seen_at: String,
}

#[derive(Deserialize)]
struct CompleteAppSettingsInput {
    locale: String,
    theme: String,
    launch_at_login_desired: bool,
    close_to_tray_notice_seen: bool,
    onboarding_completed: bool,
    last_update_check_at: Option<String>,
    updated_at: String,
}

impl TryFrom<ReplaceStateSnapshotInput> for StateSnapshot {
    type Error = crate::domain::DomainError;

    fn try_from(input: ReplaceStateSnapshotInput) -> Result<Self, Self::Error> {
        let providers = input
            .providers
            .into_iter()
            .map(|provider| {
                Ok(Provider {
                    id: ProviderId::parse(&provider.id)?,
                    kind: ProviderKind::parse(&provider.provider_kind)?,
                    built_in_key: provider.built_in_key,
                    display_name: provider.display_name,
                    base_url: provider.base_url,
                    api_key: provider.api_key.map(SecretString::new),
                    default_model: provider.default_model,
                    created_at: provider.created_at,
                    updated_at: provider.updated_at,
                })
            })
            .collect::<Result<Vec<_>, Self::Error>>()?;

        let verifications = input
            .verifications
            .into_iter()
            .map(|verification| {
                Ok(ProviderVerification {
                    provider_id: ProviderId::parse(&verification.provider_id)?,
                    combination_fingerprint: verification.combination_fingerprint,
                    verified_at: verification.verified_at,
                    contract_version: verification.contract_version,
                })
            })
            .collect::<Result<Vec<_>, Self::Error>>()?;

        let environments = input
            .environments
            .into_iter()
            .map(|environment| {
                let current_provider_id = environment
                    .current_provider_id
                    .map(|provider_id| ProviderId::parse(&provider_id))
                    .transpose()?;
                Ok(ManagedEnvironment {
                    id: EnvironmentId::parse(&environment.id)?,
                    kind: EnvironmentKind::parse(&environment.environment_kind)?,
                    platform_identity: environment.platform_identity,
                    display_name: environment.display_name,
                    current_provider_id,
                    first_seen_at: environment.first_seen_at,
                    last_seen_at: environment.last_seen_at,
                })
            })
            .collect::<Result<Vec<_>, Self::Error>>()?;

        let settings = DomainAppSettings {
            locale: Locale::parse(&input.settings.locale)?,
            theme: DomainTheme::parse(&input.settings.theme)?,
            launch_at_login_desired: input.settings.launch_at_login_desired,
            close_to_tray_notice_seen: input.settings.close_to_tray_notice_seen,
            onboarding_completed: input.settings.onboarding_completed,
            last_update_check_at: input.settings.last_update_check_at,
            updated_at: input.settings.updated_at,
        };

        StateSnapshot::new(providers, verifications, environments, settings)
    }
}

#[derive(Debug, Serialize)]
pub struct PublicStateSnapshot {
    schema_version: u32,
    counts: PublicStateCounts,
    providers: Vec<PublicProvider>,
    environments: Vec<PublicEnvironment>,
    settings: PublicCompleteAppSettings,
    state_digest: String,
}

#[derive(Debug, Serialize)]
struct PublicStateCounts {
    providers: usize,
    verified_providers: usize,
    managed_environments: usize,
}

#[derive(Debug, Serialize)]
struct PublicProvider {
    id: String,
    provider_kind: &'static str,
    verification_status: &'static str,
    combination_fingerprint: Option<String>,
}

#[derive(Debug, Serialize)]
struct PublicEnvironment {
    id: String,
    environment_kind: &'static str,
    current_provider_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct PublicCompleteAppSettings {
    locale: &'static str,
    theme: &'static str,
    launch_at_login_desired: bool,
    close_to_tray_notice_seen: bool,
    onboarding_completed: bool,
    last_update_check_at: Option<String>,
    updated_at: String,
}

impl From<&StateSnapshot> for PublicStateSnapshot {
    fn from(snapshot: &StateSnapshot) -> Self {
        let providers = snapshot
            .providers
            .iter()
            .map(|provider| {
                let verification = snapshot
                    .verifications
                    .iter()
                    .find(|verification| verification.provider_id == provider.id);
                PublicProvider {
                    id: provider.id.to_string(),
                    provider_kind: provider.kind.as_str(),
                    verification_status: if verification.is_some() {
                        "verified"
                    } else {
                        "unverified"
                    },
                    combination_fingerprint: verification
                        .map(|verification| verification.combination_fingerprint.clone()),
                }
            })
            .collect();
        let environments = snapshot
            .environments
            .iter()
            .map(|environment| PublicEnvironment {
                id: environment.id.to_string(),
                environment_kind: environment.kind.as_str(),
                current_provider_id: environment
                    .current_provider_id
                    .map(|provider_id| provider_id.to_string()),
            })
            .collect();

        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            counts: PublicStateCounts {
                providers: snapshot.providers.len(),
                verified_providers: snapshot.verifications.len(),
                managed_environments: snapshot.environments.len(),
            },
            providers,
            environments,
            settings: PublicCompleteAppSettings {
                locale: snapshot.settings.locale.as_str(),
                theme: snapshot.settings.theme.as_str(),
                launch_at_login_desired: snapshot.settings.launch_at_login_desired,
                close_to_tray_notice_seen: snapshot.settings.close_to_tray_notice_seen,
                onboarding_completed: snapshot.settings.onboarding_completed,
                last_update_check_at: snapshot.settings.last_update_check_at.clone(),
                updated_at: snapshot.settings.updated_at.clone(),
            },
            state_digest: snapshot.digest(),
        }
    }
}

#[tauri::command]
pub fn replace_state_snapshot(
    input: ReplaceStateSnapshotInput,
    store: State<'_, StateStore>,
) -> Result<PublicStateSnapshot, PublicStoreError> {
    let snapshot = StateSnapshot::try_from(input).map_err(|_| PublicStoreError::invalid_input())?;
    if !snapshot.debug_is_redacted() {
        return Err(PublicStoreError::invalid_input());
    }
    store
        .replace_snapshot(&snapshot)
        .map(|stored| PublicStateSnapshot::from(&stored))
        .map_err(Into::into)
}

#[tauri::command]
pub fn bootstrap_state_snapshot(
    store: State<'_, StateStore>,
) -> Result<PublicStateSnapshot, PublicStoreError> {
    store
        .snapshot()
        .map(|snapshot| PublicStateSnapshot::from(&snapshot))
        .map_err(Into::into)
}
