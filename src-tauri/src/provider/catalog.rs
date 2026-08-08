use rusqlite::{Connection, Error as SqliteError, TransactionBehavior, params};
use uuid::Uuid;

use crate::state::StateStore;

use super::{
    ProviderFailure, ProviderFailureCategory, ProviderSummary, VerifiedCandidate, state_unavailable,
};

pub(super) fn list_providers(
    state_store: &StateStore,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    let connection = open_catalog(state_store)?;
    let mut statement = connection
        .prepare(
            "SELECT id, name, base_url, default_model, verified_at \
             FROM providers ORDER BY name COLLATE NOCASE, id",
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
