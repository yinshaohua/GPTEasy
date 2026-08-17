use std::env;
use std::path::PathBuf;

use gpteasy_lib::provider::{
    ProviderApplication, ProviderUpdateValidationInput, ProviderValidator, ValidationTimeouts,
};
use gpteasy_lib::state::{StatePaths, StateStore};

#[tokio::test]
#[ignore = "requires GPTEASY_RUN_REAL_PROVIDER_CONTRACT=1 and an existing GPTEasy provider"]
async fn saved_provider_validation_paths_reuse_its_stored_credential() {
    if env::var("GPTEASY_RUN_REAL_PROVIDER_CONTRACT").as_deref() != Ok("1") {
        return;
    }

    let state_root = env::var_os("GPTEASY_REAL_STATE_ROOT")
        .map(PathBuf::from)
        .expect("GPTEASY_REAL_STATE_ROOT selects the existing GPTEasy state directory");
    let store = StateStore::new(StatePaths::from_root(state_root));
    let application = ProviderApplication::new(
        store,
        ProviderValidator::new(ValidationTimeouts::default()),
    );
    let providers = application
        .list_providers()
        .expect("read existing provider summaries");
    assert!(!providers.is_empty(), "the selected state has no saved provider");
    let selected_id = env::var("GPTEASY_REAL_PROVIDER_ID").ok();
    let provider = selected_id
        .as_deref()
        .and_then(|id| providers.iter().find(|provider| provider.id == id))
        .or_else(|| providers.iter().find(|provider| provider.is_current))
        .unwrap_or(&providers[0]);

    println!(
        "revalidating provider name={:?} base_url={:?} model={:?}",
        provider.name, provider.base_url, provider.default_model,
    );
    let result = application
        .revalidate_provider(
            "real-saved-provider-revalidation".to_owned(),
            provider.id.clone(),
        )
        .await;
    if let Err(failure) = &result {
        println!(
            "revalidation failed category={:?} message_id={}",
            failure.category, failure.message_id,
        );
    }
    result.expect("saved provider revalidation must reuse its stored credential");

    let result = application
        .validate_provider_update(
            "real-saved-provider-update-validation".to_owned(),
            ProviderUpdateValidationInput {
                provider_id: provider.id.clone(),
                base_url: provider.base_url.clone(),
                api_key: None,
                default_model: provider.default_model.clone(),
            },
        )
        .await;
    if let Err(failure) = &result {
        println!(
            "update validation failed category={:?} message_id={}",
            failure.category, failure.message_id,
        );
    }
    result.expect("provider update validation must reuse its stored credential");
}
