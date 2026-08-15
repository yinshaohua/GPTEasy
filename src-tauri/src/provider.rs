mod catalog;
mod linux_export;
mod validation;

pub use linux_export::{
    LinuxExportFailure, LinuxExportFailureCategory, LinuxExportResult, LinuxShell,
};

pub(crate) const DAYWAY_NAME: &str = "DayWay";
pub(crate) const DAYWAY_BASE_URL: &str = "https://dayway.site/v1";
pub(crate) const DAYWAY_RECOMMENDATION_ID: &str = "dayway";
pub(crate) const DAYWAY_WEBSITE: &str = "https://dayway.site";
const DEFAULT_VALIDATION_RECEIPT_TTL: Duration = Duration::from_secs(15 * 60);

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::environment::{EnvironmentApplication, ProviderTarget, VerifiedProviderUpdate};
use crate::state::StateStore;

pub use validation::ProviderValidator;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryInput {
    pub base_url: String,
    pub api_key: String,
}

impl fmt::Debug for DiscoveryInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveryInput")
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUpdateDiscoveryInput {
    pub provider_id: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl fmt::Debug for ProviderUpdateDiscoveryInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderUpdateDiscoveryInput")
            .field("provider_id", &self.provider_id)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderValidationInput {
    pub base_url: String,
    pub api_key: String,
    pub default_model: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUpdateValidationInput {
    pub provider_id: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: String,
}

impl fmt::Debug for ProviderUpdateValidationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderUpdateValidationInput")
            .field("provider_id", &self.provider_id)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl fmt::Debug for ProviderValidationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderValidationInput")
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .field("default_model", &self.default_model)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDiscovery {
    pub requested_base_url: String,
    pub normalized_base_url: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationEvidence {
    pub requested_base_url: String,
    pub normalized_base_url: String,
    pub default_model: String,
    pub combination_fingerprint: String,
    pub verified_at_epoch_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderValidationReceipt {
    pub validation_id: String,
    pub requested_base_url: String,
    pub normalized_base_url: String,
    pub default_model: String,
    pub combination_fingerprint: String,
    pub verified_at_epoch_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRevalidationResult {
    pub provider: ProviderSummary,
    pub validation_receipt: Option<ProviderValidationReceipt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderValidationStage {
    ModelsConfirmed,
    ResponsesStream,
    ToolRoundTrip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    pub verified_at_epoch_seconds: u64,
    pub is_current: bool,
    pub recommendation_id: Option<String>,
    pub has_recommendation_update: bool,
    #[serde(skip_serializing)]
    pub(crate) recommendation_template_base_url: Option<String>,
}

impl ProviderSummary {
    pub(crate) fn refresh_recommendation_update(&mut self) {
        self.has_recommendation_update = self.recommendation_id.as_deref()
            == Some(DAYWAY_RECOMMENDATION_ID)
            && self.recommendation_template_base_url.as_deref() != Some(DAYWAY_BASE_URL);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderApiKey {
    value: String,
}

impl fmt::Debug for ProviderApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderApiKey")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl ProviderApiKey {
    pub(crate) fn expose(&self) -> &str {
        &self.value
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ValidationTimeouts {
    pub connect: Duration,
    pub response_header: Duration,
    pub stream_read: Duration,
    pub response_overall: Duration,
}

impl Default for ValidationTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            response_header: Duration::from_secs(30),
            stream_read: Duration::from_secs(30),
            response_overall: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFailureCategory {
    SecurityPolicy,
    Transport,
    Cancelled,
    ResponseHeaderTimeout,
    FirstEventTimeout,
    StreamIdleTimeout,
    OverallTimeout,
    Authentication,
    RateLimit,
    ModelDiscovery,
    Streaming,
    ResponsesProtocol,
    ToolCall,
    ToolResult,
    InvalidInput,
    DuplicateName,
    ProviderNotFound,
    CurrentProviderProtected,
    SaveAndApplyRequired,
    SaveAndApplyFailed,
    ClipboardUnavailable,
    VerificationExpired,
    StateUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFailure {
    pub category: ProviderFailureCategory,
    pub message_id: &'static str,
}

impl ProviderFailure {
    pub(crate) fn new(category: ProviderFailureCategory, message_id: &'static str) -> Self {
        Self {
            category,
            message_id,
        }
    }
}

pub struct ProviderApplication {
    state_store: StateStore,
    validator: ProviderValidator,
    requests: Mutex<RequestRegistry>,
    verified_candidates: Mutex<HashMap<String, VerifiedCandidate>>,
    validation_receipt_ttl: Duration,
}

struct RequestRegistry {
    accepting_requests: bool,
    active: HashMap<String, CancellationToken>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedProviderUpdate {
    pub provider: ProviderSummary,
    pub environment: crate::environment::EnvironmentSnapshot,
}

#[derive(Clone)]
struct VerifiedCandidate {
    request_id: String,
    input: ProviderValidationInput,
    evidence: ValidationEvidence,
    base_url_confirmed: bool,
    created_at: Instant,
    target: ValidationTarget,
}

impl VerifiedCandidate {
    fn ensure_base_url_confirmed(&self) -> Result<(), ProviderFailure> {
        if self.base_url_confirmed {
            Ok(())
        } else {
            Err(verification_expired())
        }
    }

    fn is_expired(&self, ttl: Duration) -> bool {
        self.created_at.elapsed() >= ttl
    }
}

#[derive(Clone)]
enum ValidationTarget {
    NewProvider,
    ExistingProvider {
        provider_id: String,
        original_name: String,
        original_fingerprint: String,
    },
}

impl ProviderApplication {
    pub fn new(state_store: StateStore, validator: ProviderValidator) -> Self {
        Self::with_validation_receipt_ttl(state_store, validator, DEFAULT_VALIDATION_RECEIPT_TTL)
    }

    pub fn with_validation_receipt_ttl(
        state_store: StateStore,
        validator: ProviderValidator,
        validation_receipt_ttl: Duration,
    ) -> Self {
        Self {
            state_store,
            validator,
            requests: Mutex::new(RequestRegistry {
                accepting_requests: true,
                active: HashMap::new(),
            }),
            verified_candidates: Mutex::new(HashMap::new()),
            validation_receipt_ttl,
        }
    }

    pub async fn discover_models(
        &self,
        request_id: String,
        input: DiscoveryInput,
    ) -> Result<ModelDiscovery, ProviderFailure> {
        let cancellation = self.begin_request(&request_id)?;
        let result = self.validator.discover_models(input, cancellation).await;
        self.finish_request(&request_id);
        result
    }

    pub async fn discover_models_for_update(
        &self,
        request_id: String,
        input: ProviderUpdateDiscoveryInput,
    ) -> Result<ModelDiscovery, ProviderFailure> {
        let cancellation = self.begin_request(&request_id)?;
        let record = match catalog::get_provider(&self.state_store, &input.provider_id) {
            Ok(record) => record,
            Err(failure) => {
                self.finish_request(&request_id);
                return Err(failure);
            }
        };
        let discovery_input = DiscoveryInput {
            base_url: input.base_url,
            api_key: input
                .api_key
                .filter(|api_key| !api_key.is_empty())
                .unwrap_or(record.api_key),
        };
        let result = self
            .validator
            .discover_models(discovery_input, cancellation)
            .await;
        self.finish_request(&request_id);
        result
    }

    pub async fn validate_provider(
        &self,
        request_id: String,
        input: ProviderValidationInput,
    ) -> Result<ProviderValidationReceipt, ProviderFailure> {
        self.validate_provider_with_progress(request_id, input, |_| {})
            .await
    }

    pub async fn validate_provider_with_progress<F>(
        &self,
        request_id: String,
        input: ProviderValidationInput,
        progress: F,
    ) -> Result<ProviderValidationReceipt, ProviderFailure>
    where
        F: Fn(ProviderValidationStage),
    {
        let cancellation = self.begin_request(&request_id)?;
        let evidence = self
            .validator
            .validate_provider_with_progress(input.clone(), cancellation.clone(), progress)
            .await;
        let evidence = match evidence {
            Ok(evidence) => evidence,
            Err(failure) => {
                self.finish_request(&request_id);
                return Err(failure);
            }
        };
        self.remember_verified_candidate(
            request_id,
            cancellation,
            input,
            evidence,
            ValidationTarget::NewProvider,
        )
    }

    pub async fn validate_provider_update(
        &self,
        request_id: String,
        input: ProviderUpdateValidationInput,
    ) -> Result<ProviderValidationReceipt, ProviderFailure> {
        self.validate_provider_update_with_progress(request_id, input, |_| {})
            .await
    }

    pub async fn validate_provider_update_with_progress<F>(
        &self,
        request_id: String,
        input: ProviderUpdateValidationInput,
        progress: F,
    ) -> Result<ProviderValidationReceipt, ProviderFailure>
    where
        F: Fn(ProviderValidationStage),
    {
        let cancellation = self.begin_request(&request_id)?;
        let record = match catalog::get_provider(&self.state_store, &input.provider_id) {
            Ok(record) => record,
            Err(failure) => {
                self.finish_request(&request_id);
                return Err(failure);
            }
        };
        let validation_input = ProviderValidationInput {
            base_url: input.base_url,
            api_key: input
                .api_key
                .filter(|api_key| !api_key.is_empty())
                .unwrap_or(record.api_key),
            default_model: input.default_model,
        };
        let evidence = self
            .validator
            .validate_provider_with_progress(
                validation_input.clone(),
                cancellation.clone(),
                progress,
            )
            .await;
        let evidence = match evidence {
            Ok(evidence) => evidence,
            Err(failure) => {
                self.finish_request(&request_id);
                return Err(failure);
            }
        };
        self.remember_verified_candidate(
            request_id,
            cancellation,
            validation_input,
            evidence,
            ValidationTarget::ExistingProvider {
                provider_id: input.provider_id,
                original_name: record.summary.name,
                original_fingerprint: record.verification_fingerprint,
            },
        )
    }

    fn remember_verified_candidate(
        &self,
        request_id: String,
        cancellation: CancellationToken,
        input: ProviderValidationInput,
        evidence: ValidationEvidence,
        target: ValidationTarget,
    ) -> Result<ProviderValidationReceipt, ProviderFailure> {
        let validation_id = Uuid::new_v4().to_string();
        let mut requests = self.requests.lock().map_err(|_| state_unavailable())?;
        if cancellation.is_cancelled() {
            requests.active.remove(&request_id);
            return Err(cancelled());
        }
        let mut candidates = match self.verified_candidates.lock() {
            Ok(candidates) => candidates,
            Err(_) => {
                requests.active.remove(&request_id);
                return Err(state_unavailable());
            }
        };
        candidates.insert(
            validation_id.clone(),
            VerifiedCandidate {
                request_id: request_id.clone(),
                input,
                base_url_confirmed: evidence.requested_base_url == evidence.normalized_base_url,
                created_at: Instant::now(),
                evidence: evidence.clone(),
                target,
            },
        );
        requests.active.remove(&request_id);

        Ok(ProviderValidationReceipt {
            validation_id,
            requested_base_url: evidence.requested_base_url,
            normalized_base_url: evidence.normalized_base_url,
            default_model: evidence.default_model,
            combination_fingerprint: evidence.combination_fingerprint,
            verified_at_epoch_seconds: evidence.verified_at_epoch_seconds,
        })
    }

    pub fn cancel_request(&self, request_id: &str) -> bool {
        let active_cancelled = self
            .requests
            .lock()
            .ok()
            .and_then(|requests| requests.active.get(request_id).cloned())
            .is_some_and(|cancellation| {
                cancellation.cancel();
                true
            });
        let candidate_removed =
            self.verified_candidates
                .lock()
                .ok()
                .is_some_and(|mut candidates| {
                    let before = candidates.len();
                    candidates.retain(|_, candidate| candidate.request_id != request_id);
                    candidates.len() != before
                });
        active_cancelled || candidate_removed
    }

    pub fn shutdown_requests(&self) -> usize {
        let cancellations = self.requests.lock().map_or_else(
            |_| Vec::new(),
            |mut requests| {
                requests.accepting_requests = false;
                requests.active.values().cloned().collect::<Vec<_>>()
            },
        );
        for cancellation in &cancellations {
            cancellation.cancel();
        }
        cancellations.len()
    }

    pub fn discard_validation(&self, validation_id: &str) {
        if let Ok(mut candidates) = self.verified_candidates.lock() {
            candidates.remove(validation_id);
        }
    }

    pub fn confirm_validation_base_url(
        &self,
        validation_id: &str,
        base_url: &str,
    ) -> Result<(), ProviderFailure> {
        let mut candidates = self
            .verified_candidates
            .lock()
            .map_err(|_| state_unavailable())?;
        if candidates
            .get(validation_id)
            .is_some_and(|candidate| candidate.is_expired(self.validation_receipt_ttl))
        {
            candidates.remove(validation_id);
            return Err(verification_expired());
        }
        let candidate = candidates
            .get_mut(validation_id)
            .ok_or_else(verification_expired)?;
        if candidate.evidence.normalized_base_url != base_url {
            candidates.remove(validation_id);
            return Err(verification_expired());
        }
        candidate.base_url_confirmed = true;
        Ok(())
    }

    pub fn save_verified_provider(
        &self,
        validation_id: &str,
        name: &str,
    ) -> Result<ProviderSummary, ProviderFailure> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.name_required",
            ));
        }
        let candidate = self.valid_new_candidate(validation_id)?;
        let summary = catalog::insert_provider(&self.state_store, name, None, false, &candidate)?;
        self.discard_validation(validation_id);
        Ok(summary)
    }

    pub fn save_dayway_provider(
        &self,
        validation_id: &str,
    ) -> Result<ProviderSummary, ProviderFailure> {
        self.save_dayway_provider_with_name_conflict_confirmation(validation_id, false)
    }

    pub fn save_dayway_provider_with_name_conflict_confirmation(
        &self,
        validation_id: &str,
        confirm_name_conflict: bool,
    ) -> Result<ProviderSummary, ProviderFailure> {
        let candidate = self.valid_new_candidate(validation_id)?;
        let summary = catalog::insert_provider(
            &self.state_store,
            DAYWAY_NAME,
            Some(DAYWAY_RECOMMENDATION_ID),
            confirm_name_conflict,
            &candidate,
        )?;
        self.discard_validation(validation_id);
        Ok(summary)
    }

    pub fn save_provider_update(
        &self,
        validation_id: &str,
        provider_id: &str,
        name: &str,
    ) -> Result<ProviderSummary, ProviderFailure> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.name_required",
            ));
        }
        let candidate = self.verified_candidate(validation_id)?;
        let (original_name, original_fingerprint) = match &candidate.target {
            ValidationTarget::ExistingProvider {
                provider_id: target_id,
                original_name,
                original_fingerprint,
            } if target_id == provider_id => (original_name, original_fingerprint),
            _ => return Err(verification_expired()),
        };
        candidate.ensure_base_url_confirmed()?;
        let actual_fingerprint = combination_fingerprint(
            &candidate.evidence.normalized_base_url,
            &candidate.input.api_key,
            &candidate.input.default_model,
        );
        if actual_fingerprint != candidate.evidence.combination_fingerprint {
            self.discard_validation(validation_id);
            return Err(verification_expired());
        }

        let summary = catalog::replace_provider(
            &self.state_store,
            provider_id,
            name,
            original_name,
            original_fingerprint,
            &candidate,
        )?;
        self.discard_validation(validation_id);
        Ok(summary)
    }

    pub fn save_and_apply_provider_update(
        &self,
        environment: &EnvironmentApplication,
        validation_id: &str,
        provider_id: &str,
        name: &str,
        confirm_consumer_risk: bool,
    ) -> Result<AppliedProviderUpdate, ProviderFailure> {
        let update = self.prepare_verified_provider_update(validation_id, provider_id, name)?;
        let snapshot = environment
            .save_and_apply_provider_update(update, confirm_consumer_risk)
            .map_err(|failure| {
                ProviderFailure::new(
                    ProviderFailureCategory::SaveAndApplyFailed,
                    failure.message_id,
                )
            })?;
        let summary = snapshot.current_provider.clone().ok_or_else(|| {
            ProviderFailure::new(
                ProviderFailureCategory::SaveAndApplyFailed,
                "environment.state_unavailable",
            )
        })?;
        self.discard_validation(validation_id);
        Ok(AppliedProviderUpdate {
            provider: summary,
            environment: snapshot,
        })
    }

    fn prepare_verified_provider_update(
        &self,
        validation_id: &str,
        provider_id: &str,
        name: &str,
    ) -> Result<VerifiedProviderUpdate, ProviderFailure> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.name_required",
            ));
        }
        let candidate = self.verified_candidate(validation_id)?;
        let (original_name, original_fingerprint) = match &candidate.target {
            ValidationTarget::ExistingProvider {
                provider_id: target_id,
                original_name,
                original_fingerprint,
            } if target_id == provider_id => (original_name.clone(), original_fingerprint.clone()),
            _ => return Err(verification_expired()),
        };
        candidate.ensure_base_url_confirmed()?;
        let actual_fingerprint = combination_fingerprint(
            &candidate.evidence.normalized_base_url,
            &candidate.input.api_key,
            &candidate.input.default_model,
        );
        if actual_fingerprint != candidate.evidence.combination_fingerprint {
            self.discard_validation(validation_id);
            return Err(verification_expired());
        }
        let existing = catalog::get_provider(&self.state_store, provider_id)?;
        if existing.summary.recommendation_id.is_none() && name.eq_ignore_ascii_case(DAYWAY_NAME) {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.recommended_name_reserved",
            ));
        }
        if existing.summary.recommendation_id.is_some() && name != existing.summary.name {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.recommended_name_fixed",
            ));
        }
        let recommendation_template_base_url = if existing.summary.recommendation_id.is_some()
            && candidate.evidence.normalized_base_url == DAYWAY_BASE_URL
        {
            Some(DAYWAY_BASE_URL.to_owned())
        } else {
            existing.summary.recommendation_template_base_url
        };
        let provider = ProviderTarget::new(
            provider_id.to_owned(),
            name.to_owned(),
            candidate.evidence.normalized_base_url.clone(),
            candidate.input.api_key.clone(),
            candidate.input.default_model.clone(),
            candidate.evidence.verified_at_epoch_seconds,
            candidate.evidence.combination_fingerprint.clone(),
            existing.summary.recommendation_id,
            recommendation_template_base_url,
        );
        Ok(VerifiedProviderUpdate::new(
            provider,
            original_name,
            original_fingerprint,
        ))
    }

    pub async fn revalidate_provider(
        &self,
        request_id: String,
        provider_id: String,
    ) -> Result<ProviderRevalidationResult, ProviderFailure> {
        self.revalidate_provider_with_progress(request_id, provider_id, |_| {})
            .await
    }

    pub async fn revalidate_provider_with_progress<F>(
        &self,
        request_id: String,
        provider_id: String,
        progress: F,
    ) -> Result<ProviderRevalidationResult, ProviderFailure>
    where
        F: Fn(ProviderValidationStage),
    {
        let cancellation = self.begin_request(&request_id)?;
        let record = match catalog::get_provider(&self.state_store, &provider_id) {
            Ok(record) => record,
            Err(failure) => {
                self.finish_request(&request_id);
                return Err(failure);
            }
        };
        let input = ProviderValidationInput {
            base_url: record.summary.base_url.clone(),
            api_key: record.api_key,
            default_model: record.summary.default_model.clone(),
        };
        let evidence = match self
            .validator
            .validate_provider_with_progress(input.clone(), cancellation.clone(), progress)
            .await
        {
            Ok(evidence) => evidence,
            Err(failure) => {
                self.finish_request(&request_id);
                return Err(failure);
            }
        };
        if evidence.requested_base_url == evidence.normalized_base_url {
            let result = catalog::record_revalidation(
                &self.state_store,
                &provider_id,
                &record.verification_fingerprint,
                &evidence,
            );
            self.finish_request(&request_id);
            let provider = result?;
            return Ok(ProviderRevalidationResult {
                provider,
                validation_receipt: None,
            });
        }
        let provider = record.summary.clone();
        let receipt = self.remember_verified_candidate(
            request_id,
            cancellation,
            input,
            evidence,
            ValidationTarget::ExistingProvider {
                provider_id,
                original_name: record.summary.name,
                original_fingerprint: record.verification_fingerprint,
            },
        )?;
        Ok(ProviderRevalidationResult {
            provider,
            validation_receipt: Some(receipt),
        })
    }

    pub fn reveal_provider_api_key(
        &self,
        provider_id: &str,
    ) -> Result<ProviderApiKey, ProviderFailure> {
        catalog::get_provider(&self.state_store, provider_id).map(|record| ProviderApiKey {
            value: record.api_key,
        })
    }

    pub fn list_providers(&self) -> Result<Vec<ProviderSummary>, ProviderFailure> {
        catalog::list_providers(&self.state_store)
    }

    pub fn export_linux_script(
        &self,
        shell: LinuxShell,
        destination: &std::path::Path,
        confirm_overwrite: bool,
    ) -> Result<LinuxExportResult, LinuxExportFailure> {
        linux_export::export(&self.state_store, shell, destination, confirm_overwrite)
    }

    pub fn rename_provider(
        &self,
        provider_id: &str,
        name: &str,
    ) -> Result<ProviderSummary, ProviderFailure> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.name_required",
            ));
        }
        catalog::rename_provider(&self.state_store, provider_id, name)
    }

    pub fn delete_provider(&self, provider_id: &str) -> Result<(), ProviderFailure> {
        catalog::delete_provider(&self.state_store, provider_id)
    }

    pub fn reorder_providers(
        &self,
        provider_ids: &[String],
    ) -> Result<Vec<ProviderSummary>, ProviderFailure> {
        catalog::reorder_providers(&self.state_store, provider_ids)
    }

    fn begin_request(&self, request_id: &str) -> Result<CancellationToken, ProviderFailure> {
        if request_id.trim().is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.request_id_required",
            ));
        }
        let cancellation = CancellationToken::new();
        let mut requests = self.requests.lock().map_err(|_| state_unavailable())?;
        if !requests.accepting_requests {
            return Err(cancelled());
        }
        if requests.active.contains_key(request_id) {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.request_already_running",
            ));
        }
        requests
            .active
            .insert(request_id.to_owned(), cancellation.clone());
        Ok(cancellation)
    }

    fn valid_new_candidate(
        &self,
        validation_id: &str,
    ) -> Result<VerifiedCandidate, ProviderFailure> {
        let candidate = self.verified_candidate(validation_id)?;
        if !matches!(candidate.target, ValidationTarget::NewProvider) {
            return Err(verification_expired());
        }
        candidate.ensure_base_url_confirmed()?;
        let actual_fingerprint = combination_fingerprint(
            &candidate.evidence.normalized_base_url,
            &candidate.input.api_key,
            &candidate.input.default_model,
        );
        if actual_fingerprint != candidate.evidence.combination_fingerprint {
            self.discard_validation(validation_id);
            return Err(verification_expired());
        }
        Ok(candidate)
    }

    fn verified_candidate(
        &self,
        validation_id: &str,
    ) -> Result<VerifiedCandidate, ProviderFailure> {
        let mut candidates = self
            .verified_candidates
            .lock()
            .map_err(|_| state_unavailable())?;
        if candidates
            .get(validation_id)
            .is_some_and(|candidate| candidate.is_expired(self.validation_receipt_ttl))
        {
            candidates.remove(validation_id);
            return Err(verification_expired());
        }
        candidates
            .get(validation_id)
            .cloned()
            .ok_or_else(verification_expired)
    }

    fn finish_request(&self, request_id: &str) {
        if let Ok(mut requests) = self.requests.lock() {
            requests.active.remove(request_id);
        }
    }
}

fn combination_fingerprint(base_url: &str, api_key: &str, model: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-provider-combination-v1\0");
    hasher.update(base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(model.as_bytes());
    hasher.update(b"\0");
    hasher.update(api_key.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn verification_expired() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::VerificationExpired,
        "provider.verification_expired",
    )
}

fn state_unavailable() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::StateUnavailable,
        "provider.state_unavailable",
    )
}

fn cancelled() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::Cancelled,
        "provider.request_cancelled",
    )
}
