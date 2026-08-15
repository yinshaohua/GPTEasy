#![cfg(windows)]

use std::path::PathBuf;

use gpteasy_lib::provider::{
    ProviderApplication, ProviderSummary, ProviderValidator, ValidationTimeouts,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use gpteasy_lib::wsl::{WslApplication, WslAvailability};

struct ProviderRestore<'a> {
    application: &'a WslApplication,
    environment_id: String,
    provider_id: String,
    active: bool,
}

impl Drop for ProviderRestore<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let Ok(environments) = self.application.list() else {
            return;
        };
        let Some(environment) = environments
            .iter()
            .find(|environment| environment.environment_id == self.environment_id)
        else {
            return;
        };
        if environment
            .current_provider
            .as_ref()
            .map(|provider| &provider.id)
            == Some(&self.provider_id)
        {
            return;
        }
        let _ = self.application.apply_provider(
            &self.environment_id,
            &self.provider_id,
            &environment.revision,
            true,
        );
    }
}

fn assert_provider(actual: &ProviderSummary, expected: &ProviderSummary) {
    assert_eq!(actual.id, expected.id);
    assert_eq!(actual.name, expected.name);
    assert_eq!(actual.base_url, expected.base_url);
    assert_eq!(actual.default_model, expected.default_model);
}

#[test]
#[ignore = "mutates the real GPTEasy state and one real WSL2 distribution before restoring it"]
fn real_installed_catalog_switches_wsl_and_restores_the_original_provider() {
    assert_eq!(
        std::env::var("GPTEASY_RUN_REAL_WSL_SWITCH").as_deref(),
        Ok("1"),
        "set GPTEASY_RUN_REAL_WSL_SWITCH=1 to confirm the real switch",
    );
    let state_root = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA"))
        .join("com.gpteasy.desktop");
    let store = StateStore::new(StatePaths::from_root(state_root));
    let catalog = ProviderApplication::new(
        store.clone(),
        ProviderValidator::new(ValidationTimeouts::default()),
    )
    .list_providers()
    .expect("list installed providers");
    assert!(
        catalog.len() >= 2,
        "the real switch requires two stored providers"
    );

    let application = WslApplication::new(store);
    let environment = application
        .list()
        .expect("list real WSL2 environments")
        .into_iter()
        .find(|environment| environment.availability == WslAvailability::Manageable)
        .expect("one manageable WSL2 environment");
    assert!(
        environment.running,
        "the real switch test requires an already running distribution"
    );
    let original = environment
        .current_provider
        .clone()
        .expect("the real WSL2 environment must already have a provider to restore");
    let alternate = catalog
        .iter()
        .find(|provider| provider.id != original.id)
        .cloned()
        .expect("an alternate stored provider");

    let switched = application
        .apply_provider(
            &environment.environment_id,
            &alternate.id,
            &environment.revision,
            true,
        )
        .expect("apply alternate provider");
    let mut restore = ProviderRestore {
        application: &application,
        environment_id: environment.environment_id.clone(),
        provider_id: original.id.clone(),
        active: true,
    };
    assert_provider(
        switched
            .environment
            .current_provider
            .as_ref()
            .expect("alternate provider is current"),
        &alternate,
    );

    let restored = application
        .apply_provider(
            &environment.environment_id,
            &original.id,
            &switched.environment.revision,
            true,
        )
        .expect("restore original provider");
    restore.active = false;
    assert_provider(
        restored
            .environment
            .current_provider
            .as_ref()
            .expect("original provider is current again"),
        &original,
    );
    assert!(restored.environment.running);
    println!(
        "restored {} after switching from {} to {}",
        environment.display_name, original.name, alternate.name
    );
}
