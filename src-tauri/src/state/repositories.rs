use rusqlite::{params, Connection, Transaction};

use crate::domain::{
    AppSettings, EnvironmentId, EnvironmentKind, Locale, ManagedEnvironment, Provider, ProviderId,
    ProviderKind, ProviderVerification, SecretString, StateSnapshot, Theme,
};

use super::StoreError;

pub(crate) fn replace_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &StateSnapshot,
) -> Result<(), StoreError> {
    transaction.execute("DELETE FROM provider_verifications", [])?;
    transaction.execute("DELETE FROM managed_environments", [])?;
    transaction.execute("DELETE FROM providers", [])?;

    {
        let mut statement = transaction.prepare(
            "INSERT INTO providers (
                 id, provider_kind, built_in_key, display_name, base_url,
                 api_key, default_model, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for provider in &snapshot.providers {
            statement.execute(params![
                provider.id.to_string(),
                provider.kind.as_str(),
                provider.built_in_key,
                provider.display_name,
                provider.base_url,
                provider.api_key.as_ref().map(SecretString::expose_secret),
                provider.default_model,
                provider.created_at,
                provider.updated_at,
            ])?;
        }
    }

    {
        let mut statement = transaction.prepare(
            "INSERT INTO provider_verifications (
                 provider_id, combination_fingerprint, verified_at, contract_version
             ) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for verification in &snapshot.verifications {
            statement.execute(params![
                verification.provider_id.to_string(),
                verification.combination_fingerprint,
                verification.verified_at,
                verification.contract_version,
            ])?;
        }
    }

    {
        let mut statement = transaction.prepare(
            "INSERT INTO managed_environments (
                 id, environment_kind, platform_identity, display_name,
                 current_provider_id, first_seen_at, last_seen_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for environment in &snapshot.environments {
            let current_provider_id = environment
                .current_provider_id
                .map(|provider_id| provider_id.to_string());
            statement.execute(params![
                environment.id.to_string(),
                environment.kind.as_str(),
                environment.platform_identity,
                environment.display_name,
                current_provider_id,
                environment.first_seen_at,
                environment.last_seen_at,
            ])?;
        }
    }

    let changed = transaction.execute(
        "UPDATE app_settings
         SET locale = ?1,
             theme = ?2,
             launch_at_login_desired = ?3,
             close_to_tray_notice_seen = ?4,
             onboarding_completed = ?5,
             last_update_check_at = ?6,
             updated_at = ?7
         WHERE singleton_id = 1",
        params![
            snapshot.settings.locale.as_str(),
            snapshot.settings.theme.as_str(),
            i64::from(snapshot.settings.launch_at_login_desired),
            i64::from(snapshot.settings.close_to_tray_notice_seen),
            i64::from(snapshot.settings.onboarding_completed),
            snapshot.settings.last_update_check_at,
            snapshot.settings.updated_at,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::ContractMismatch);
    }
    Ok(())
}

pub(crate) fn read_snapshot(connection: &Connection) -> Result<StateSnapshot, StoreError> {
    let providers = read_providers(connection)?;
    let verifications = read_verifications(connection)?;
    let environments = read_environments(connection)?;
    let settings = read_complete_settings(connection)?;
    StateSnapshot::new(providers, verifications, environments, settings)
        .map_err(|_| StoreError::ContractMismatch)
}

fn read_providers(connection: &Connection) -> Result<Vec<Provider>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT id, provider_kind, built_in_key, display_name, base_url,
                api_key, default_model, created_at, updated_at
         FROM providers
         ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
        ))
    })?;

    rows.map(|row| {
        let (
            id,
            kind,
            built_in_key,
            display_name,
            base_url,
            api_key,
            default_model,
            created_at,
            updated_at,
        ) = row?;
        Ok(Provider {
            id: ProviderId::parse(&id).map_err(|_| StoreError::ContractMismatch)?,
            kind: ProviderKind::parse(&kind).map_err(|_| StoreError::ContractMismatch)?,
            built_in_key,
            display_name,
            base_url,
            api_key: api_key.map(SecretString::new),
            default_model,
            created_at,
            updated_at,
        })
    })
    .collect()
}

fn read_verifications(connection: &Connection) -> Result<Vec<ProviderVerification>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT provider_id, combination_fingerprint, verified_at, contract_version
         FROM provider_verifications
         ORDER BY provider_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    rows.map(|row| {
        let (provider_id, combination_fingerprint, verified_at, contract_version) = row?;
        Ok(ProviderVerification {
            provider_id: ProviderId::parse(&provider_id)
                .map_err(|_| StoreError::ContractMismatch)?,
            combination_fingerprint,
            verified_at,
            contract_version,
        })
    })
    .collect()
}

fn read_environments(connection: &Connection) -> Result<Vec<ManagedEnvironment>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT id, environment_kind, platform_identity, display_name,
                current_provider_id, first_seen_at, last_seen_at
         FROM managed_environments
         ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    rows.map(|row| {
        let (
            id,
            kind,
            platform_identity,
            display_name,
            current_provider_id,
            first_seen_at,
            last_seen_at,
        ) = row?;
        let current_provider_id = current_provider_id
            .map(|provider_id| ProviderId::parse(&provider_id))
            .transpose()
            .map_err(|_| StoreError::ContractMismatch)?;
        Ok(ManagedEnvironment {
            id: EnvironmentId::parse(&id).map_err(|_| StoreError::ContractMismatch)?,
            kind: EnvironmentKind::parse(&kind).map_err(|_| StoreError::ContractMismatch)?,
            platform_identity,
            display_name,
            current_provider_id,
            first_seen_at,
            last_seen_at,
        })
    })
    .collect()
}

fn read_complete_settings(connection: &Connection) -> Result<AppSettings, StoreError> {
    let record = connection.query_row(
        "SELECT locale, theme, launch_at_login_desired,
                close_to_tray_notice_seen, onboarding_completed,
                last_update_check_at, updated_at
         FROM app_settings
         WHERE singleton_id = 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        },
    )?;
    let (
        locale,
        theme,
        launch_at_login_desired,
        close_to_tray_notice_seen,
        onboarding_completed,
        last_update_check_at,
        updated_at,
    ) = record;
    Ok(AppSettings {
        locale: Locale::parse(&locale).map_err(|_| StoreError::ContractMismatch)?,
        theme: Theme::parse(&theme).map_err(|_| StoreError::ContractMismatch)?,
        launch_at_login_desired: launch_at_login_desired != 0,
        close_to_tray_notice_seen: close_to_tray_notice_seen != 0,
        onboarding_completed: onboarding_completed != 0,
        last_update_check_at,
        updated_at,
    })
}
