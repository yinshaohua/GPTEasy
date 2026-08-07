use std::{collections::HashSet, fmt};

use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const PROVIDER_FINGERPRINT_DOMAIN: &[u8] = b"gpteasy-provider-combination-v1\0";
const STATE_SNAPSHOT_DOMAIN: &str = "gpteasy-state-snapshot-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(Uuid);

impl ProviderId {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| DomainError::InvalidState)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvironmentId(Uuid);

impl EnvironmentId {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| DomainError::InvalidState)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for EnvironmentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    BuiltInRecommended,
    Custom,
}

impl ProviderKind {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "built_in_recommended" => Ok(Self::BuiltInRecommended),
            "custom" => Ok(Self::Custom),
            _ => Err(DomainError::InvalidState),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuiltInRecommended => "built_in_recommended",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub id: ProviderId,
    pub kind: ProviderKind,
    pub built_in_key: Option<String>,
    pub display_name: String,
    pub base_url: Option<String>,
    pub api_key: Option<SecretString>,
    pub default_model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderVerification {
    pub provider_id: ProviderId,
    pub combination_fingerprint: String,
    pub verified_at: String,
    pub contract_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvironmentKind {
    NativeCodex,
    Wsl2,
}

impl EnvironmentKind {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "native_codex" => Ok(Self::NativeCodex),
            "wsl2" => Ok(Self::Wsl2),
            _ => Err(DomainError::InvalidState),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativeCodex => "native_codex",
            Self::Wsl2 => "wsl2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedEnvironment {
    pub id: EnvironmentId,
    pub kind: EnvironmentKind,
    pub platform_identity: String,
    pub display_name: String,
    pub current_provider_id: Option<ProviderId>,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    System,
    ZhCn,
    EnUs,
}

impl Locale {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "system" => Ok(Self::System),
            "zh-CN" => Ok(Self::ZhCn),
            "en-US" => Ok(Self::EnUs),
            _ => Err(DomainError::InvalidState),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(DomainError::InvalidState),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub locale: Locale,
    pub theme: Theme,
    pub launch_at_login_desired: bool,
    pub close_to_tray_notice_seen: bool,
    pub onboarding_completed: bool,
    pub last_update_check_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSnapshot {
    pub providers: Vec<Provider>,
    pub verifications: Vec<ProviderVerification>,
    pub environments: Vec<ManagedEnvironment>,
    pub settings: AppSettings,
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid authoritative state")]
    InvalidState,
}

impl StateSnapshot {
    pub fn new(
        mut providers: Vec<Provider>,
        mut verifications: Vec<ProviderVerification>,
        mut environments: Vec<ManagedEnvironment>,
        settings: AppSettings,
    ) -> Result<Self, DomainError> {
        providers.sort_by_key(|provider| provider.id);
        verifications.sort_by_key(|verification| verification.provider_id);
        environments.sort_by_key(|environment| environment.id);

        validate_settings(&settings)?;
        let provider_ids = validate_providers(&providers)?;
        validate_verifications(&providers, &provider_ids, &verifications)?;
        validate_environments(&provider_ids, &environments)?;

        Ok(Self {
            providers,
            verifications,
            environments,
            settings,
        })
    }

    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hash_string(&mut hasher, STATE_SNAPSHOT_DOMAIN);

        hash_count(&mut hasher, self.providers.len());
        for provider in &self.providers {
            hash_string(&mut hasher, &provider.id.to_string());
            hasher.update([match provider.kind {
                ProviderKind::BuiltInRecommended => 0,
                ProviderKind::Custom => 1,
            }]);
            hash_optional_string(&mut hasher, provider.built_in_key.as_deref());
            hash_string(&mut hasher, &provider.display_name);
            hash_optional_string(&mut hasher, provider.base_url.as_deref());
            hash_optional_string(
                &mut hasher,
                provider.api_key.as_ref().map(SecretString::expose_secret),
            );
            hash_optional_string(&mut hasher, provider.default_model.as_deref());
            hash_string(&mut hasher, &provider.created_at);
            hash_string(&mut hasher, &provider.updated_at);
        }

        hash_count(&mut hasher, self.verifications.len());
        for verification in &self.verifications {
            hash_string(&mut hasher, &verification.provider_id.to_string());
            hash_string(&mut hasher, &verification.combination_fingerprint);
            hash_string(&mut hasher, &verification.verified_at);
            hash_string(&mut hasher, &verification.contract_version);
        }

        hash_count(&mut hasher, self.environments.len());
        for environment in &self.environments {
            hash_string(&mut hasher, &environment.id.to_string());
            hasher.update([match environment.kind {
                EnvironmentKind::NativeCodex => 0,
                EnvironmentKind::Wsl2 => 1,
            }]);
            hash_string(&mut hasher, &environment.platform_identity);
            hash_string(&mut hasher, &environment.display_name);
            let current_provider_id = environment.current_provider_id.map(|id| id.to_string());
            hash_optional_string(&mut hasher, current_provider_id.as_deref());
            hash_string(&mut hasher, &environment.first_seen_at);
            hash_string(&mut hasher, &environment.last_seen_at);
        }

        hasher.update([match self.settings.locale {
            Locale::System => 0,
            Locale::ZhCn => 1,
            Locale::EnUs => 2,
        }]);
        hasher.update([match self.settings.theme {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        }]);
        hasher.update([u8::from(self.settings.launch_at_login_desired)]);
        hasher.update([u8::from(self.settings.close_to_tray_notice_seen)]);
        hasher.update([u8::from(self.settings.onboarding_completed)]);
        hash_optional_string(&mut hasher, self.settings.last_update_check_at.as_deref());
        hash_string(&mut hasher, &self.settings.updated_at);

        lowercase_hex(&hasher.finalize())
    }

    pub fn debug_is_redacted(&self) -> bool {
        let rendered = format!("{self:?}");
        self.providers.iter().all(|provider| {
            provider
                .api_key
                .as_ref()
                .is_none_or(|secret| !rendered.contains(secret.expose_secret()))
        })
    }
}

fn validate_providers(providers: &[Provider]) -> Result<HashSet<ProviderId>, DomainError> {
    let mut ids = HashSet::with_capacity(providers.len());
    let mut built_in_keys = HashSet::new();
    for provider in providers {
        if !ids.insert(provider.id)
            || !valid_required(&provider.display_name)
            || !valid_required(&provider.created_at)
            || !valid_required(&provider.updated_at)
            || !valid_optional(provider.base_url.as_deref())
            || !valid_optional(provider.default_model.as_deref())
            || !valid_optional(provider.api_key.as_ref().map(SecretString::expose_secret))
        {
            return Err(DomainError::InvalidState);
        }

        match (&provider.kind, &provider.built_in_key) {
            (ProviderKind::BuiltInRecommended, Some(key))
                if valid_required(key) && built_in_keys.insert(key.as_str()) => {}
            (ProviderKind::Custom, None) => {}
            _ => return Err(DomainError::InvalidState),
        }
    }
    Ok(ids)
}

fn validate_verifications(
    providers: &[Provider],
    provider_ids: &HashSet<ProviderId>,
    verifications: &[ProviderVerification],
) -> Result<(), DomainError> {
    let mut verified_ids = HashSet::with_capacity(verifications.len());
    for verification in verifications {
        if !provider_ids.contains(&verification.provider_id)
            || !verified_ids.insert(verification.provider_id)
            || !is_lowercase_sha256(&verification.combination_fingerprint)
            || !valid_required(&verification.verified_at)
            || !valid_required(&verification.contract_version)
        {
            return Err(DomainError::InvalidState);
        }

        let provider = providers
            .iter()
            .find(|provider| provider.id == verification.provider_id)
            .ok_or(DomainError::InvalidState)?;
        if provider_combination_fingerprint(provider).as_deref()
            != Some(verification.combination_fingerprint.as_str())
        {
            return Err(DomainError::InvalidState);
        }
    }
    Ok(())
}

fn validate_environments(
    provider_ids: &HashSet<ProviderId>,
    environments: &[ManagedEnvironment],
) -> Result<(), DomainError> {
    let mut ids = HashSet::with_capacity(environments.len());
    let mut platform_identities = HashSet::with_capacity(environments.len());
    for environment in environments {
        if !ids.insert(environment.id)
            || !platform_identities
                .insert((environment.kind, environment.platform_identity.as_str()))
            || !valid_required(&environment.platform_identity)
            || !valid_required(&environment.display_name)
            || !valid_required(&environment.first_seen_at)
            || !valid_required(&environment.last_seen_at)
            || environment
                .current_provider_id
                .is_some_and(|provider_id| !provider_ids.contains(&provider_id))
        {
            return Err(DomainError::InvalidState);
        }
    }
    Ok(())
}

fn validate_settings(settings: &AppSettings) -> Result<(), DomainError> {
    if !valid_required(&settings.updated_at)
        || !valid_optional(settings.last_update_check_at.as_deref())
    {
        return Err(DomainError::InvalidState);
    }
    Ok(())
}

fn valid_required(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains('\0')
}

fn valid_optional(value: Option<&str>) -> bool {
    value.is_none_or(valid_required)
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn provider_combination_fingerprint(provider: &Provider) -> Option<String> {
    let base_url = provider.base_url.as_deref()?;
    let default_model = provider.default_model.as_deref()?;
    let api_key = provider.api_key.as_ref()?.expose_secret();
    let mut hasher = Sha256::new();
    hasher.update(PROVIDER_FINGERPRINT_DOMAIN);
    hasher.update(base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(default_model.as_bytes());
    hasher.update(b"\0");
    hasher.update(api_key.as_bytes());
    Some(lowercase_hex(&hasher.finalize()))
}

fn hash_count(hasher: &mut Sha256, count: usize) {
    hasher.update((count as u64).to_be_bytes());
}

fn hash_string(hasher: &mut Sha256, value: &str) {
    hash_count(hasher, value.len());
    hasher.update(value.as_bytes());
}

fn hash_optional_string(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_string(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
