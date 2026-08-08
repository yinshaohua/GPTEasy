use rusqlite::{Connection, Error as SqliteError, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use crate::state::StateStore;

use super::{
    ProviderFailure, ProviderFailureCategory, ProviderSummary, ValidationEvidence,
    VerifiedCandidate, state_unavailable, verification_expired,
};

pub(super) struct ProviderRecord {
    pub summary: ProviderSummary,
    pub api_key: String,
    pub verification_fingerprint: String,
}

pub(super) fn list_providers(
    state_store: &StateStore,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    let connection = open_catalog(state_store)?;
    let mut statement = connection
        .prepare(
            "SELECT p.id, p.name, p.base_url, p.default_model, p.verified_at, \
                    EXISTS(\
                        SELECT 1 FROM last_applied_state current \
                        WHERE current.singleton = 1 \
                          AND current.mode = 'provider' \
                          AND current.provider_id = p.id\
                    ) \
             FROM providers p ORDER BY p.name COLLATE NOCASE, p.id",
        )
        .map_err(|_| state_unavailable())?;
    statement
        .query_map([], |row| {
            let verified_at = row.get::<_, String>(4)?;
            Ok(ProviderSummary {
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
            })
        })
        .map_err(|_| state_unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| state_unavailable())
}

pub(super) fn insert_provider(
    state_store: &StateStore,
    name: &str,
    candidate: &VerifiedCandidate,
) -> Result<ProviderSummary, ProviderFailure> {
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
    if existing_names
        .iter()
        .any(|existing| existing.to_lowercase() == normalized_name)
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::DuplicateName,
            "provider.name_duplicate",
        ));
    }

    let summary = ProviderSummary {
        id: Uuid::new_v4().to_string(),
        name: name.to_owned(),
        base_url: candidate.evidence.normalized_base_url.clone(),
        default_model: candidate.input.default_model.clone(),
        verified_at_epoch_seconds: candidate.evidence.verified_at_epoch_seconds,
        is_current: false,
    };
    transaction
        .execute(
            "INSERT INTO providers (\
                id, name, base_url, api_key, default_model, verified_at, verification_fingerprint\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                summary.id,
                summary.name,
                summary.base_url,
                candidate.input.api_key,
                summary.default_model,
                summary.verified_at_epoch_seconds.to_string(),
                candidate.evidence.combination_fingerprint,
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
    transaction.commit().map_err(|_| state_unavailable())?;
    Ok(summary)
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
    transaction
        .execute("DELETE FROM providers WHERE id = ?1", [provider_id])
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
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
) -> Result<ProviderSummary, ProviderFailure> {
    let mut connection = open_catalog(state_store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let record = find_provider_record(&transaction, provider_id)?.ok_or_else(provider_not_found)?;
    if record.summary.is_current {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::SaveAndApplyRequired,
            "provider.save_and_apply_required",
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
                verified_at = ?5, verification_fingerprint = ?6 \
             WHERE id = ?7",
            params![
                name,
                candidate.evidence.normalized_base_url,
                candidate.input.api_key,
                candidate.input.default_model,
                candidate.evidence.verified_at_epoch_seconds.to_string(),
                candidate.evidence.combination_fingerprint,
                provider_id,
            ],
        )
        .map_err(map_write_failure)?;
    transaction.commit().map_err(|_| state_unavailable())?;
    Ok(ProviderSummary {
        id: provider_id.to_owned(),
        name: name.to_owned(),
        base_url: candidate.evidence.normalized_base_url.clone(),
        default_model: candidate.input.default_model.clone(),
        verified_at_epoch_seconds: candidate.evidence.verified_at_epoch_seconds,
        is_current: false,
    })
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
    transaction.commit().map_err(|_| state_unavailable())?;
    Ok(ProviderSummary {
        verified_at_epoch_seconds: evidence.verified_at_epoch_seconds,
        ..record.summary
    })
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
                    ) \
             FROM providers p WHERE p.id = ?1",
            [provider_id],
            |row| {
                let verified_at = row.get::<_, String>(4)?;
                Ok(ProviderSummary {
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
                })
            },
        )
        .optional()
        .map_err(|_| state_unavailable())
}

fn find_provider_record(
    connection: &Connection,
    provider_id: &str,
) -> Result<Option<ProviderRecord>, ProviderFailure> {
    connection
        .query_row(
            "SELECT p.id, p.name, p.base_url, p.api_key, p.default_model, p.verified_at, \
                    p.verification_fingerprint, \
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
                Ok(ProviderRecord {
                    summary: ProviderSummary {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        base_url: row.get(2)?,
                        default_model: row.get(4)?,
                        verified_at_epoch_seconds: verified_at.parse().map_err(|error| {
                            SqliteError::FromSqlConversionFailure(
                                5,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?,
                        is_current: row.get::<_, i64>(7)? == 1,
                    },
                    api_key: row.get(3)?,
                    verification_fingerprint: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|_| state_unavailable())
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
