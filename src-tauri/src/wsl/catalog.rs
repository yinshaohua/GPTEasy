use super::*;

pub(super) struct CatalogMetadata {
    pub relative: String,
    pub artifact: String,
    pub sha256: String,
    pub fingerprint: String,
    pub policy: String,
}

pub(super) fn catalog_failure(message: &'static str) -> WslFailure {
    WslFailure::new(WslFailureCategory::NeedsAttention, message)
}

pub(super) fn catalog_artifact(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 prefix")).to_string()
}

fn metadata_value<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let mut values = text.lines().filter_map(|line| {
        line.trim_end_matches('\r')
            .strip_prefix(prefix)
            .map(str::trim)
    });
    let value = values.next()?;
    (values.next().is_none() && !value.is_empty()).then_some(value)
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn digest_is_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(super) fn catalog_relative_from_config(config: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(config).ok()?;
    let relative = metadata_value(text, "# GPTEasy model-catalog-file:")?;
    let parts = relative.split('/').collect::<Vec<_>>();
    if parts.len() != 4
        || parts[0] != ".gpteasy-shell"
        || parts[1] != "model-catalogs"
        || !safe_component(parts[2])
        || !parts[3].ends_with(".json")
        || Uuid::parse_str(parts[3].strip_suffix(".json")?).is_err()
    {
        return None;
    }
    Some(relative)
}

pub(super) fn catalog_metadata(config: &[u8]) -> Option<CatalogMetadata> {
    let text = std::str::from_utf8(config).ok()?;
    let start = text.find("# >>> GPTEasy managed provider >>>")?;
    let end = text.find("# <<< GPTEasy managed provider <<<")?;
    if start >= end {
        return None;
    }
    let text = &text[start..end];
    if metadata_value(text, "# GPTEasy schema-version:")? != "2"
        || metadata_value(text, "# GPTEasy model-catalog-protocol:")? != CATALOG_PROTOCOL
    {
        return None;
    }
    let source = metadata_value(text, "# GPTEasy source-id:")?;
    let artifact = metadata_value(text, "# GPTEasy model-catalog-artifact:")?;
    let relative = catalog_relative_from_config(config)?;
    let sha256 = metadata_value(text, "# GPTEasy model-catalog-sha256:")?;
    let fingerprint = metadata_value(text, "# GPTEasy model-catalog-provider-fingerprint:")?;
    let policy = metadata_value(text, "# GPTEasy model-catalog-policy:")?;
    if !safe_component(source)
        || Uuid::parse_str(artifact).is_err()
        || relative != format!(".gpteasy-shell/model-catalogs/{source}/{artifact}.json")
        || !digest_is_valid(sha256)
        || !digest_is_valid(fingerprint)
        || !safe_component(policy)
    {
        return None;
    }
    Some(CatalogMetadata {
        relative: relative.to_owned(),
        artifact: artifact.to_owned(),
        sha256: sha256.to_owned(),
        fingerprint: fingerprint.to_owned(),
        policy: policy.to_owned(),
    })
}

pub(super) fn inspect_artifacts(artifacts: &WslArtifacts) -> ActualManagedState {
    let observed = inspect_actual_managed_state(
        artifacts.config.as_deref(),
        artifacts.credentials.as_deref(),
    );
    let ActualManagedState::Current {
        provider_id,
        base_url,
        model,
        ..
    } = &observed
    else {
        return observed;
    };
    let config = artifacts.config.as_deref().expect("current config");
    let Some(metadata) = catalog_metadata(config) else {
        return managed_conflict("wsl.catalog_binding_invalid");
    };
    let text = std::str::from_utf8(config).expect("parsed config");
    let start = text
        .find("# >>> GPTEasy managed provider >>>")
        .expect("managed block");
    let end = text
        .find("# <<< GPTEasy managed provider <<<")
        .expect("managed block");
    let block = &text[start..end];
    let Ok(block_doc) = block.parse::<toml_edit::DocumentMut>() else {
        return managed_conflict("wsl.catalog_reference_invalid");
    };
    let Some(home) = artifacts.codex_home.as_deref() else {
        return managed_conflict("wsl.catalog_reference_invalid");
    };
    if !home.starts_with('/')
        || block_doc
            .get("model_catalog_json")
            .and_then(|value| value.as_str())
            != Some(format!("{home}/{}", metadata.relative).as_str())
    {
        return managed_conflict("wsl.catalog_reference_invalid");
    }
    let credential = artifacts
        .credentials
        .as_deref()
        .expect("validated credential");
    let Ok(key) = std::str::from_utf8(credential) else {
        return managed_conflict("wsl.credentials_invalid");
    };
    if crate::provider::combination_fingerprint(base_url, key, model) != metadata.fingerprint {
        return managed_conflict("wsl.catalog_binding_invalid");
    }
    // Provider identity is carried by the block; it never comes from local SQLite.
    if Uuid::parse_str(provider_id).is_err() {
        return managed_conflict("wsl.catalog_binding_invalid");
    }
    let Some(bytes) = artifacts.catalog.as_deref() else {
        return managed_conflict("wsl.catalog_missing");
    };
    if hash_bytes(bytes) != metadata.sha256 {
        return managed_conflict("wsl.catalog_corrupt");
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return managed_conflict("wsl.catalog_corrupt");
    };
    let Some(models) = value.get("models").and_then(|value| value.as_array()) else {
        return managed_conflict("wsl.catalog_corrupt");
    };
    let ids = models
        .iter()
        .map(|entry| {
            entry
                .get("slug")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .collect::<Option<Vec<_>>>();
    let Some(ids) = ids else {
        return managed_conflict("wsl.catalog_corrupt");
    };
    if !ids.iter().any(|id| id == model) {
        return managed_conflict("wsl.catalog_binding_invalid");
    }
    let Ok(expected) = model_catalog::render(&ids, model) else {
        return managed_conflict("wsl.catalog_corrupt");
    };
    if metadata.policy == model_catalog::REASONING_SELECTOR_POLICY
        && serde_json::from_slice::<serde_json::Value>(&expected)
            .ok()
            .as_ref()
            != Some(&value)
    {
        return managed_conflict("wsl.catalog_corrupt");
    }
    observed
}
