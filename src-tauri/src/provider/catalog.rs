use rusqlite::{Connection, Error as SqliteError, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use crate::state::StateStore;

use super::reasoning::{self, ReasoningSelection};
use super::reasoning_capability::{
    CodexMetadataSnapshot, apply_dayway_ds_compatibility, apply_deepseek_compatibility,
};
use super::{
    DAYWAY_BASE_URL, DAYWAY_NAME, ProviderFailure, ProviderFailureCategory, ProviderSummary,
    ValidationEvidence, VerifiedCandidate, combination_fingerprint, state_unavailable,
    verification_expired,
};

pub(super) struct ProviderRecord {
    pub summary: ProviderSummary,
    pub api_key: String,
    pub verification_fingerprint: String,
    pub reasoning_selection: ReasoningSelection,
    pub discovered_models: Vec<String>,
    pub capability_snapshot: Option<CodexMetadataSnapshot>,
}

pub(super) fn list_providers(
    state_store: &StateStore,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    let connection = open_catalog(state_store)?;
    list_providers_from_connection(&connection)
}

pub(super) fn list_provider_records(
    state_store: &StateStore,
) -> Result<Vec<ProviderRecord>, ProviderFailure> {
    let connection = open_catalog(state_store)?;
    let summaries = list_providers_from_connection(&connection)?;
    summaries
        .into_iter()
        .map(|summary| {
            find_provider_record(&connection, &summary.id)?.ok_or_else(provider_not_found)
        })
        .collect()
}

pub(super) fn insert_provider(
    state_store: &StateStore,
    name: &str,
    recommendation_id: Option<&str>,
    confirm_name_conflict: bool,
    candidate: &VerifiedCandidate,
) -> Result<ProviderSummary, ProviderFailure> {
    if recommendation_id.is_none() && name.eq_ignore_ascii_case(DAYWAY_NAME) {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.recommended_name_reserved",
        ));
    }
    let mut connection = open_catalog(state_store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let existing_names = {
        let mut statement = transaction
            .prepare("SELECT name FROM providers")
            .map_err(|_| state_unavailable())?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| state_unavailable())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| state_unavailable())?
    };
    let normalized_name = name.to_lowercase();
    let duplicate_name = existing_names
        .iter()
        .any(|existing| existing.to_lowercase() == normalized_name);
    if duplicate_name && recommendation_id.is_some() && !confirm_name_conflict {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.recommended_name_conflict",
        ));
    }
    if duplicate_name && recommendation_id.is_none() {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::DuplicateName,
            "provider.name_duplicate",
        ));
    }
    if recommendation_id.is_some()
        && transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM providers WHERE recommendation_id = ?1)",
                [recommendation_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| state_unavailable())?
            == 1
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.recommendation_exists",
        ));
    }
    if duplicate_name {
        let replacement = available_legacy_dayway_name(&existing_names);
        transaction
            .execute(
                "UPDATE providers SET name = ?1 WHERE name = ?2 COLLATE NOCASE AND recommendation_id IS NULL",
                params![replacement, DAYWAY_NAME],
            )
            .map_err(map_write_failure)?;
    }

    let summary = ProviderSummary {
        id: Uuid::new_v4().to_string(),
        name: name.to_owned(),
        base_url: candidate.evidence.normalized_base_url.clone(),
        default_model: candidate.input.default_model.clone(),
        verified_at_epoch_seconds: candidate.evidence.verified_at_epoch_seconds,
        is_current: false,
        recommendation_id: recommendation_id.map(str::to_owned),
        has_recommendation_update: false,
        recommendation_template_base_url: recommendation_id.map(|_| DAYWAY_BASE_URL.to_owned()),
    };
    transaction
        .execute(
            "INSERT INTO providers (\
                id, name, base_url, api_key, default_model, verified_at, verification_fingerprint, sort_order, recommendation_id, recommendation_template_base_url\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE((SELECT MAX(sort_order) + 1 FROM providers), 0), ?8, ?9)",
            params![
                summary.id,
                summary.name,
                summary.base_url,
                candidate.input.api_key,
                summary.default_model,
                summary.verified_at_epoch_seconds.to_string(),
                candidate.evidence.combination_fingerprint,
                summary.recommendation_id,
                summary.recommendation_template_base_url,
            ],
        )
        .map_err(|error| match error {
            SqliteError::SqliteFailure(_, Some(message)) if message.contains("providers.name") => {
                ProviderFailure::new(
                    ProviderFailureCategory::DuplicateName,
                    "provider.name_duplicate",
                )
            }
            _ => state_unavailable(),
        })?;
    store_model_catalog(
        &transaction,
        &summary.id,
        &candidate.evidence.combination_fingerprint,
        &candidate.evidence.discovered_models,
        None,
    )?;
    transaction.commit().map_err(|_| state_unavailable())?;
    Ok(summary)
}

fn available_legacy_dayway_name(existing_names: &[String]) -> String {
    let base = format!("{DAYWAY_NAME} (原供应商)");
    if !existing_names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&base))
    {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base} {suffix}");
        if !existing_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
    }
    unreachable!()
}

pub(super) fn rename_provider(
    state_store: &StateStore,
    provider_id: &str,
    name: &str,
) -> Result<ProviderSummary, ProviderFailure> {
    let mut connection = open_catalog(state_store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let mut summary = find_provider(&transaction, provider_id)?.ok_or_else(provider_not_found)?;
    if summary.recommendation_id.is_none() && name.eq_ignore_ascii_case(DAYWAY_NAME) {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.recommended_name_reserved",
        ));
    }
    if summary.recommendation_id.is_some() && name != summary.name {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.recommended_name_fixed",
        ));
    }
    ensure_name_available(&transaction, Some(provider_id), name)?;
    transaction
        .execute(
            "UPDATE providers SET name = ?1 WHERE id = ?2",
            params![name, provider_id],
        )
        .map_err(map_write_failure)?;
    transaction.commit().map_err(|_| state_unavailable())?;
    summary.name = name.to_owned();
    Ok(summary)
}

pub(super) fn delete_provider(
    state_store: &StateStore,
    provider_id: &str,
) -> Result<(), ProviderFailure> {
    let mut connection = open_catalog(state_store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let summary = find_provider(&transaction, provider_id)?.ok_or_else(provider_not_found)?;
    if summary.is_current {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::CurrentProviderProtected,
            "provider.current_delete_forbidden",
        ));
    }
    let used_by_wsl = transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM wsl_environments WHERE current_provider_id = ?1
                UNION ALL
                SELECT 1 FROM wsl_pending_operation WHERE target_provider_id = ?1
            )",
            [provider_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| state_unavailable())?;
    if used_by_wsl {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::CurrentProviderProtected,
            "provider.wsl_current_delete_forbidden",
        ));
    }
    transaction
        .execute("DELETE FROM providers WHERE id = ?1", [provider_id])
        .map_err(|_| state_unavailable())?;
    compact_order(&transaction)?;
    transaction.commit().map_err(|_| state_unavailable())
}

pub(super) fn reorder_providers(
    state_store: &StateStore,
    provider_ids: &[String],
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    let mut connection = open_catalog(state_store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let mut statement = transaction
        .prepare("SELECT id FROM providers ORDER BY recommendation_id IS NULL, sort_order, rowid")
        .map_err(|_| state_unavailable())?;
    let existing = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| state_unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| state_unavailable())?;
    drop(statement);
    if provider_ids.len() != existing.len()
        || provider_ids
            .iter()
            .any(|id| !existing.iter().any(|value| value == id))
        || {
            let mut unique = provider_ids.to_vec();
            unique.sort();
            unique.dedup();
            unique.len() != provider_ids.len()
        }
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.order_invalid",
        ));
    }
    if let Some(recommended) = transaction
        .query_row(
            "SELECT id FROM providers WHERE recommendation_id IS NOT NULL",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| state_unavailable())?
        && provider_ids.first() != Some(&recommended)
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.order_invalid",
        ));
    }
    // Move rows into a temporary range before assigning the requested positions.
    transaction
        .execute(
            "UPDATE providers SET sort_order = sort_order + ?1",
            [existing.len() as i64],
        )
        .map_err(|_| state_unavailable())?;
    for (index, provider_id) in provider_ids.iter().enumerate() {
        transaction
            .execute(
                "UPDATE providers SET sort_order = ?1 WHERE id = ?2",
                params![index as i64, provider_id],
            )
            .map_err(|_| state_unavailable())?;
    }
    let summaries = list_providers_from_connection(&transaction)?;
    transaction.commit().map_err(|_| state_unavailable())?;
    Ok(summaries)
}

fn compact_order(transaction: &rusqlite::Transaction<'_>) -> Result<(), ProviderFailure> {
    let ids = {
        let mut statement = transaction
            .prepare(
                "SELECT id FROM providers ORDER BY recommendation_id IS NULL, sort_order, rowid",
            )
            .map_err(|_| state_unavailable())?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| state_unavailable())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| state_unavailable())?
    };
    let offset = ids.len() as i64 + 1;
    transaction
        .execute(
            "UPDATE providers SET sort_order = sort_order + ?1",
            [offset],
        )
        .map_err(|_| state_unavailable())?;
    for (index, provider_id) in ids.iter().enumerate() {
        transaction
            .execute(
                "UPDATE providers SET sort_order = ?1 WHERE id = ?2",
                params![index as i64, provider_id],
            )
            .map_err(|_| state_unavailable())?;
    }
    Ok(())
}

fn list_providers_from_connection(
    connection: &Connection,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    let mut statement = connection
        .prepare(
            "SELECT p.id, p.name, p.base_url, p.default_model, p.verified_at, EXISTS(SELECT 1 FROM last_applied_state current WHERE current.singleton = 1 AND current.mode = 'provider' AND current.provider_id = p.id), p.recommendation_id, p.recommendation_template_base_url FROM providers p ORDER BY p.recommendation_id IS NULL, p.sort_order, p.rowid",
        )
        .map_err(|_| state_unavailable())?;
    statement
        .query_map([], |row| {
            let verified_at = row.get::<_, String>(4)?;
            let mut summary = ProviderSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                default_model: row.get(3)?,
                verified_at_epoch_seconds: verified_at.parse().map_err(|error| {
                    SqliteError::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                is_current: row.get::<_, i64>(5)? == 1,
                recommendation_id: row.get(6)?,
                has_recommendation_update: false,
                recommendation_template_base_url: row.get(7)?,
            };
            summary.refresh_recommendation_update();
            Ok(summary)
        })
        .map_err(|_| state_unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| state_unavailable())
}

pub(super) fn get_provider(
    state_store: &StateStore,
    provider_id: &str,
) -> Result<ProviderRecord, ProviderFailure> {
    let connection = open_catalog(state_store)?;
    find_provider_record(&connection, provider_id)?.ok_or_else(provider_not_found)
}

pub(super) fn replace_provider(
    state_store: &StateStore,
    provider_id: &str,
    name: &str,
    original_name: &str,
    original_fingerprint: &str,
    candidate: &VerifiedCandidate,
    allow_recorded_current: bool,
) -> Result<ProviderSummary, ProviderFailure> {
    let mut connection = open_catalog(state_store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let record = find_provider_record(&transaction, provider_id)?.ok_or_else(provider_not_found)?;
    if record.summary.recommendation_id.is_none() && name.eq_ignore_ascii_case(DAYWAY_NAME) {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.recommended_name_reserved",
        ));
    }
    if record.summary.is_current && !allow_recorded_current {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::SaveAndApplyRequired,
            "provider.save_and_apply_required",
        ));
    }
    if record.summary.recommendation_id.is_some() && name != record.summary.name {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidInput,
            "provider.recommended_name_fixed",
        ));
    }
    if record.summary.name != original_name
        || record.verification_fingerprint != original_fingerprint
    {
        return Err(verification_expired());
    }
    ensure_name_available(&transaction, Some(provider_id), name)?;
    transaction
        .execute(
            "UPDATE providers SET \
                name = ?1, base_url = ?2, api_key = ?3, default_model = ?4, \
                verified_at = ?5, verification_fingerprint = ?6, \
                recommendation_template_base_url = CASE \
                    WHEN recommendation_id IS NOT NULL AND ?2 = ?8 THEN ?8 \
                    ELSE recommendation_template_base_url END \
             WHERE id = ?7",
            params![
                name,
                candidate.evidence.normalized_base_url,
                candidate.input.api_key,
                candidate.input.default_model,
                candidate.evidence.verified_at_epoch_seconds.to_string(),
                candidate.evidence.combination_fingerprint,
                provider_id,
                DAYWAY_BASE_URL,
            ],
        )
        .map_err(map_write_failure)?;
    store_model_catalog(
        &transaction,
        provider_id,
        &candidate.evidence.combination_fingerprint,
        &candidate.evidence.discovered_models,
        None,
    )?;
    transaction.commit().map_err(|_| state_unavailable())?;
    let mut summary = ProviderSummary {
        id: provider_id.to_owned(),
        name: name.to_owned(),
        base_url: candidate.evidence.normalized_base_url.clone(),
        default_model: candidate.input.default_model.clone(),
        verified_at_epoch_seconds: candidate.evidence.verified_at_epoch_seconds,
        is_current: false,
        recommendation_id: record.summary.recommendation_id,
        has_recommendation_update: false,
        recommendation_template_base_url: if candidate.evidence.normalized_base_url
            == DAYWAY_BASE_URL
        {
            Some(DAYWAY_BASE_URL.to_owned())
        } else {
            record.summary.recommendation_template_base_url
        },
    };
    summary.refresh_recommendation_update();
    Ok(summary)
}

pub(super) fn record_revalidation(
    state_store: &StateStore,
    provider_id: &str,
    original_fingerprint: &str,
    evidence: &ValidationEvidence,
) -> Result<ProviderSummary, ProviderFailure> {
    let mut connection = open_catalog(state_store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let record = find_provider_record(&transaction, provider_id)?.ok_or_else(provider_not_found)?;
    if record.verification_fingerprint != original_fingerprint {
        return Err(verification_expired());
    }
    let changed = transaction
        .execute(
            "UPDATE providers SET verified_at = ?1, verification_fingerprint = ?2 \
             WHERE id = ?3 AND verification_fingerprint = ?4",
            params![
                evidence.verified_at_epoch_seconds.to_string(),
                evidence.combination_fingerprint,
                provider_id,
                original_fingerprint,
            ],
        )
        .map_err(|_| state_unavailable())?;
    if changed != 1 {
        return Err(verification_expired());
    }
    store_model_catalog(
        &transaction,
        provider_id,
        &evidence.combination_fingerprint,
        &evidence.discovered_models,
        None,
    )?;
    transaction.commit().map_err(|_| state_unavailable())?;
    Ok(ProviderSummary {
        verified_at_epoch_seconds: evidence.verified_at_epoch_seconds,
        ..record.summary
    })
}

fn store_model_catalog(
    transaction: &rusqlite::Transaction<'_>,
    provider_id: &str,
    fingerprint: &str,
    models: &[String],
    capability_snapshot: Option<&CodexMetadataSnapshot>,
) -> Result<(), ProviderFailure> {
    let models_json = serde_json::to_string(models).map_err(|_| state_unavailable())?;
    let capability_snapshot_json = capability_snapshot
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| state_unavailable())?;
    transaction
        .execute(
            "INSERT INTO provider_model_catalog(
                provider_id, verification_fingerprint, models_json, capability_snapshot_json
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider_id) DO UPDATE SET
                verification_fingerprint=excluded.verification_fingerprint,
                models_json=excluded.models_json,
                capability_snapshot_json=excluded.capability_snapshot_json",
            params![
                provider_id,
                fingerprint,
                models_json,
                capability_snapshot_json
            ],
        )
        .map_err(|_| state_unavailable())?;
    Ok(())
}

fn find_provider(
    connection: &Connection,
    provider_id: &str,
) -> Result<Option<ProviderSummary>, ProviderFailure> {
    connection
        .query_row(
            "SELECT p.id, p.name, p.base_url, p.default_model, p.verified_at, \
                    EXISTS(\
                        SELECT 1 FROM last_applied_state current \
                        WHERE current.singleton = 1 \
                          AND current.mode = 'provider' \
                          AND current.provider_id = p.id\
                    ), p.recommendation_id, p.recommendation_template_base_url \
             FROM providers p WHERE p.id = ?1",
            [provider_id],
            |row| {
                let verified_at = row.get::<_, String>(4)?;
                let mut summary = ProviderSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    base_url: row.get(2)?,
                    default_model: row.get(3)?,
                    verified_at_epoch_seconds: verified_at.parse().map_err(|error| {
                        SqliteError::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    is_current: row.get::<_, i64>(5)? == 1,
                    recommendation_id: row.get(6)?,
                    has_recommendation_update: false,
                    recommendation_template_base_url: row.get(7)?,
                };
                summary.refresh_recommendation_update();
                Ok(summary)
            },
        )
        .optional()
        .map_err(|_| state_unavailable())
}

fn find_provider_record(
    connection: &Connection,
    provider_id: &str,
) -> Result<Option<ProviderRecord>, ProviderFailure> {
    let record = connection
        .query_row(
            "SELECT p.id, p.name, p.base_url, p.api_key, p.default_model, p.verified_at, \
                    p.verification_fingerprint, p.recommendation_id, p.recommendation_template_base_url, \
                    EXISTS(\
                        SELECT 1 FROM last_applied_state current \
                        WHERE current.singleton = 1 \
                          AND current.mode = 'provider' \
                          AND current.provider_id = p.id\
                    ) \
             FROM providers p WHERE p.id = ?1",
            [provider_id],
            |row| {
                let verified_at = row.get::<_, String>(5)?;
                let name = row.get::<_, String>(1)?;
                let base_url = row.get::<_, String>(2)?;
                Ok(ProviderRecord {
                    summary: {
                        let mut summary = ProviderSummary {
                        id: row.get(0)?,
                        name: name.clone(),
                        base_url: base_url.clone(),
                        default_model: row.get(4)?,
                        verified_at_epoch_seconds: verified_at.parse().map_err(|error| {
                            SqliteError::FromSqlConversionFailure(
                                5,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?,
                        is_current: row.get::<_, i64>(9)? == 1,
                        recommendation_id: row.get(7)?,
                        has_recommendation_update: false,
                        recommendation_template_base_url: row.get(8)?,
                        };
                        summary.refresh_recommendation_update();
                        summary
                    },
                    api_key: row.get(3)?,
                    verification_fingerprint: row.get(6)?,
                    reasoning_selection: reasoning::for_provider(&name, &base_url),
                    discovered_models: Vec::new(),
                    capability_snapshot: None,
                })
            },
        )
        .optional()
        .map_err(|_| state_unavailable())?;
    let Some(mut record) = record else {
        return Ok(None);
    };
    let expected_fingerprint = combination_fingerprint(
        &record.summary.base_url,
        &record.api_key,
        &record.summary.default_model,
    );
    let catalog = connection
        .query_row(
            "SELECT verification_fingerprint, models_json, capability_snapshot_json
             FROM provider_model_catalog WHERE provider_id = ?1",
            [&record.summary.id],
            |row| {
                let fingerprint: String = row.get(0)?;
                let models_json: String = row.get(1)?;
                let snapshot_json: Option<String> = row.get(2)?;
                Ok((fingerprint, models_json, snapshot_json))
            },
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|(fingerprint, models_json, snapshot_json)| {
            decode_model_catalog(
                &fingerprint,
                &expected_fingerprint,
                &models_json,
                snapshot_json.as_deref(),
                &record.summary.default_model,
            )
        });
    if let Some(catalog) = catalog {
        record.discovered_models = catalog.0;
        record.capability_snapshot = catalog.1;
        // 导出目录和审计共用兼容规则，只修饰读取副本，保留数据库中的原始探测证据。
        if let Some(snapshot) = record.capability_snapshot.as_mut() {
            match record.reasoning_selection.rule_id.as_str() {
                reasoning::DEEPSEEK_HIGH_RULE => {
                    apply_deepseek_compatibility(snapshot, &record.summary.default_model)
                }
                reasoning::DAYWAY_DS_DEEPSEEK_RULE => apply_dayway_ds_compatibility(snapshot),
                _ => {}
            }
        }
    }
    Ok(Some(record))
}

fn decode_model_catalog(
    fingerprint: &str,
    expected_fingerprint: &str,
    models_json: &str,
    snapshot_json: Option<&str>,
    default_model: &str,
) -> Option<(Vec<String>, Option<CodexMetadataSnapshot>)> {
    if fingerprint != expected_fingerprint {
        return None;
    }
    // 合法的 [] 是可信的空发现结果；损坏或缺失的 JSON 则不是空目录。
    let models = serde_json::from_str::<Vec<String>>(models_json).ok()?;
    let mut rendered_models = models.clone();
    if !rendered_models.iter().any(|model| model == default_model) {
        rendered_models.push(default_model.to_owned());
    }
    let snapshot = snapshot_json
        .and_then(|json| serde_json::from_str::<CodexMetadataSnapshot>(json).ok())
        .filter(|snapshot| snapshot.covers_exact_models(&rendered_models));
    Some((models, snapshot))
}

#[cfg(test)]
mod model_catalog_decode_tests {
    use super::decode_model_catalog;
    use crate::provider::reasoning_capability::{
        CapabilitySource, CapabilityState, CodexMetadataSnapshot, CodexModelMetadata,
        SNAPSHOT_SCHEMA_VERSION, SnapshotStatus,
    };

    fn snapshot(models: &[&str]) -> String {
        serde_json::to_string(&CodexMetadataSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            target_environment: "native".to_owned(),
            codex_version: Some("0.160.1".to_owned()),
            executable_fingerprint: Some("sha256:test".to_owned()),
            observed_at_epoch_seconds: 1,
            status: SnapshotStatus::Complete,
            models: models
                .iter()
                .map(|model| CodexModelMetadata {
                    model_id: (*model).to_owned(),
                    state: CapabilityState::KnownNonEmpty,
                    default_reasoning_effort: Some("medium".to_owned()),
                    supported_reasoning_efforts: vec!["medium".to_owned()],
                    source: CapabilitySource::CodexBuiltin,
                    source_id: "codex.model/list".to_owned(),
                })
                .collect(),
        })
        .expect("snapshot json")
    }

    #[test]
    fn mismatched_fingerprint_does_not_reuse_catalog_or_snapshot() {
        assert_eq!(
            decode_model_catalog(
                "stored",
                "expected",
                r#"["model-a"]"#,
                Some(&snapshot(&["model-a", "default-model"])),
                "default-model",
            ),
            None
        );
    }

    #[test]
    fn corrupt_models_json_is_not_a_valid_empty_catalog() {
        assert_eq!(
            decode_model_catalog("expected", "expected", "not-json", None, "default-model",),
            None
        );
    }

    #[test]
    fn corrupt_snapshot_keeps_models_but_safely_drops_capabilities() {
        assert_eq!(
            decode_model_catalog(
                "expected",
                "expected",
                r#"["model-a"]"#,
                Some("not-json"),
                "default-model",
            ),
            Some((vec!["model-a".to_owned()], None))
        );
    }

    #[test]
    fn partial_snapshot_drops_capabilities_for_the_whole_rendered_model_set() {
        assert_eq!(
            decode_model_catalog(
                "expected",
                "expected",
                r#"["model-a","model-b"]"#,
                Some(&snapshot(&["model-a", "default-model"])),
                "default-model",
            ),
            Some((vec!["model-a".to_owned(), "model-b".to_owned()], None))
        );
    }

    #[test]
    fn legacy_provider_without_snapshot_remains_a_valid_unknown_catalog() {
        assert_eq!(
            decode_model_catalog(
                "expected",
                "expected",
                r#"["model-a"]"#,
                None,
                "default-model",
            ),
            Some((vec!["model-a".to_owned()], None))
        );
    }

    #[test]
    fn legal_empty_array_is_preserved_as_an_empty_catalog() {
        assert_eq!(
            decode_model_catalog("expected", "expected", "[]", None, "default-model",),
            Some((Vec::new(), None))
        );
    }

    #[test]
    fn complete_snapshot_must_also_cover_default_model() {
        let decoded = decode_model_catalog(
            "expected",
            "expected",
            r#"["model-a"]"#,
            Some(&snapshot(&["model-a", "default-model"])),
            "default-model",
        )
        .expect("catalog");
        assert!(decoded.1.is_some());
    }
}

#[cfg(test)]
mod model_catalog_storage_tests {
    use super::store_model_catalog;
    use crate::state::{StatePaths, StateStore};
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    #[test]
    fn new_verification_clears_stale_snapshot() {
        let temp = TempDir::new().expect("temp dir");
        let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
        assert!(store.bootstrap().is_ready());
        let mut connection = Connection::open(store.paths().database()).expect("database");
        let snapshot = r#"{"schemaVersion":1,"targetEnvironment":"native"}"#;
        connection
            .execute(
                "INSERT INTO providers(
                    id, name, base_url, api_key, default_model, verified_at, verification_fingerprint
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    "provider-1",
                    "Provider 1",
                    "https://provider.example/v1",
                    "secret",
                    "old-model",
                    "1",
                    "old-fingerprint",
                ],
            )
            .expect("provider");
        connection
            .execute(
                "INSERT INTO provider_model_catalog(
                    provider_id, verification_fingerprint, models_json, capability_snapshot_json
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    "provider-1",
                    "old-fingerprint",
                    r#"["old-model"]"#,
                    snapshot
                ],
            )
            .expect("existing catalog");
        {
            let transaction = connection.transaction().expect("transaction");
            store_model_catalog(
                &transaction,
                "provider-1",
                "new-fingerprint",
                &["new-model".to_owned()],
                None,
            )
            .expect("update catalog");
            transaction.commit().expect("commit");
        }
        let stored: Option<String> = connection
            .query_row(
                "SELECT capability_snapshot_json FROM provider_model_catalog WHERE provider_id = ?1",
                ["provider-1"],
                |row| row.get(0),
            )
            .expect("stored snapshot");
        assert_eq!(stored, None);
    }
}
fn ensure_name_available(
    connection: &Connection,
    provider_id: Option<&str>,
    name: &str,
) -> Result<(), ProviderFailure> {
    let mut statement = connection
        .prepare("SELECT id, name FROM providers")
        .map_err(|_| state_unavailable())?;
    let names = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| state_unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| state_unavailable())?;
    let normalized_name = name.to_lowercase();
    if names.iter().any(|(existing_id, existing_name)| {
        Some(existing_id.as_str()) != provider_id && existing_name.to_lowercase() == normalized_name
    }) {
        Err(ProviderFailure::new(
            ProviderFailureCategory::DuplicateName,
            "provider.name_duplicate",
        ))
    } else {
        Ok(())
    }
}

fn map_write_failure(error: SqliteError) -> ProviderFailure {
    match error {
        SqliteError::SqliteFailure(_, Some(message)) if message.contains("providers.name") => {
            ProviderFailure::new(
                ProviderFailureCategory::DuplicateName,
                "provider.name_duplicate",
            )
        }
        _ => state_unavailable(),
    }
}

fn provider_not_found() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::ProviderNotFound,
        "provider.not_found",
    )
}

fn open_catalog(state_store: &StateStore) -> Result<Connection, ProviderFailure> {
    if !state_store.bootstrap().is_ready() {
        return Err(state_unavailable());
    }
    let connection =
        Connection::open(state_store.paths().database()).map_err(|_| state_unavailable())?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
        .map_err(|_| state_unavailable())?;
    Ok(connection)
}
