#[test]
fn catalog_apply_rejects_invalid_snapshots_before_starting_or_locking() {
    for mutation in [
        "DELETE FROM provider_model_catalog",
        "UPDATE provider_model_catalog SET models_json = '[]'",
        "UPDATE provider_model_catalog SET models_json = 'broken'",
        "UPDATE provider_model_catalog SET verification_fingerprint = 'different'",
        "UPDATE providers SET api_key = 'changed-without-verification'",
    ] {
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: None,
                credentials: None,
                catalog: None,
                codex_home: Some("/home/test/.codex".to_owned()),
            },
        ));
        let (_temp, store, app) = application(runtime.clone());
        let environment = app.list().unwrap().remove(0);
        Connection::open(store.paths().database())
            .unwrap()
            .execute(mutation, [])
            .unwrap();
        let error = app
            .apply_provider(
                &environment.environment_id,
                "22222222-2222-4222-8222-222222222222",
                &environment.revision,
                true,
            )
            .unwrap_err();
        assert_eq!(
            error.message_id, "wsl.catalog_snapshot_invalid",
            "{mutation}"
        );
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.lock_acquisitions.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.writes.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn catalog_bundle_delivers_every_model_and_common_reasoning_metadata() {
    let provider = provider("22222222-2222-4222-8222-222222222222");
    let artifacts = schema_v2_artifacts(&provider, "desktop-test");
    let config = artifacts.config.as_ref().unwrap();
    let bundle = bundle_bytes(config, provider.api_key.as_bytes(), &provider.catalog);
    let (written_config, credentials, catalog) = decode_test_bundle(&bundle);
    assert_eq!(written_config, *config);
    assert_eq!(credentials, provider.api_key.as_bytes());
    assert_eq!(catalog, provider.catalog);
    let value: serde_json::Value = serde_json::from_slice(&catalog).unwrap();
    let models = value["models"].as_array().unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model["slug"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["model-a", "model-b", "vendor-new"]
    );
    for model in models {
        assert_eq!(model["default_reasoning_level"], "high");
        assert_eq!(
            model["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["effort"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["low", "medium", "high", "xhigh"]
        );
    }
    assert!(matches!(
        inspect_artifacts(&artifacts),
        ActualManagedState::Current { .. }
    ));
}

#[test]
fn catalog_reader_distinguishes_missing_corrupt_binding_and_home_conflicts() {
    let provider = provider("22222222-2222-4222-8222-222222222222");
    let original = schema_v2_artifacts(&provider, "shell-test");
    let conflict = |artifacts: &WslArtifacts, expected| {
        assert!(
            matches!(inspect_artifacts(artifacts), ActualManagedState::Conflict { message_id }
            if message_id == expected),
            "expected {expected}"
        );
    };
    let mut changed = original.clone();
    changed.catalog = None;
    conflict(&changed, "wsl.catalog_missing");
    changed.catalog = Some(b"broken".to_vec());
    conflict(&changed, "wsl.catalog_corrupt");
    changed = original.clone();
    changed.credentials = Some(b"another-provider-key".to_vec());
    conflict(&changed, "wsl.catalog_binding_invalid");
    changed = original.clone();
    changed.codex_home = Some("/home/test/custom-codex".to_owned());
    conflict(&changed, "wsl.catalog_reference_invalid");
    changed = original.clone();
    let metadata = catalog_metadata(changed.config.as_ref().unwrap()).unwrap();
    let mut json: serde_json::Value =
        serde_json::from_slice(changed.catalog.as_ref().unwrap()).unwrap();
    json["models"][0]["default_reasoning_level"] = serde_json::json!("low");
    let bytes = serde_json::to_vec_pretty(&json).unwrap();
    changed.config = Some(
        String::from_utf8(changed.config.take().unwrap())
            .unwrap()
            .replace(&metadata.sha256, &hash_bytes(&bytes))
            .into_bytes(),
    );
    changed.catalog = Some(bytes);
    conflict(&changed, "wsl.catalog_corrupt");
    changed = original.clone();
    let text = String::from_utf8(changed.config.take().unwrap()).unwrap();
    let line = text
        .lines()
        .find(|line| line.starts_with("# GPTEasy model-catalog-policy:"))
        .unwrap();
    changed.config = Some(format!("{line}\n{text}").into_bytes());
    // Duplicate metadata outside the block must never become an alternate binding.
    conflict(&changed, "wsl.catalog_binding_invalid");
}

#[test]
fn catalog_refresh_compares_snapshot_and_policy_without_treating_source_or_name_as_provider_updates()
 {
    let provider = provider("22222222-2222-4222-8222-222222222222");
    let mut running = probe();
    running.running = true;
    let runtime = Arc::new(FakeRuntime::new(
        running,
        schema_v2_artifacts(&provider, "shell-source"),
    ));
    let (_temp, store, app) = application(runtime.clone());
    assert_eq!(
        app.list().unwrap()[0].configuration_state,
        WslConfigurationState::Current
    );
    let mut renamed = provider.clone();
    renamed.name = "Renamed only".to_owned();
    *runtime.artifacts.lock().unwrap() = schema_v2_artifacts(&renamed, "another-source");
    let refreshed = app.list().unwrap().remove(0);
    assert_eq!(
        refreshed.configuration_state,
        WslConfigurationState::Current
    );
    assert!(refreshed.pending_restart);
    let mut old = provider.clone();
    old.catalog = model_catalog::render(&["model-a".to_owned()], "model-a").unwrap();
    *runtime.artifacts.lock().unwrap() = schema_v2_artifacts(&old, "old-snapshot");
    let refreshed = app.list().unwrap().remove(0);
    assert_eq!(
        refreshed.configuration_state,
        WslConfigurationState::Updated
    );
    assert_eq!(
        refreshed.message_id.as_deref(),
        Some("wsl.catalog_snapshot_updated")
    );
    let mut artifacts = schema_v2_artifacts(&provider, "old-policy");
    artifacts.config = Some(
        String::from_utf8(artifacts.config.take().unwrap())
            .unwrap()
            .replace(
                "common-reasoning-selector-v1",
                "common-reasoning-selector-v0",
            )
            .into_bytes(),
    );
    *runtime.artifacts.lock().unwrap() = artifacts;
    let refreshed = app.list().unwrap().remove(0);
    assert_eq!(
        refreshed.configuration_state,
        WslConfigurationState::Updated
    );
    assert_eq!(
        refreshed.message_id.as_deref(),
        Some("wsl.catalog_policy_outdated")
    );
    assert_eq!(runtime.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        Connection::open(store.paths().database())
            .unwrap()
            .query_row("SELECT count(*) FROM provider_model_catalog", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn catalog_saga_keeps_conflicting_third_artifact_and_lock_at_every_persisted_commit_stage() {
    for point in [
        WslFailurePoint::AfterPrepared,
        WslFailurePoint::AfterArtifactsReplaced,
        WslFailurePoint::AfterStateCommitted,
    ] {
        for missing in [false, true] {
            let provider_id = "22222222-2222-4222-8222-222222222222";
            let mut running = probe();
            running.running = true;
            let runtime = Arc::new(FakeRuntime::new(
                running,
                schema_v2_artifacts(&provider(provider_id), "shell-before"),
            ));
            let (_temp, store, _) = application(runtime.clone());
            let app = WslApplication::with_dependencies(
                store.clone(),
                runtime.clone(),
                Arc::new(InterruptAt(point)),
            );
            let environment = app.list().unwrap().remove(0);
            assert_eq!(
                app.apply_provider(
                    &environment.environment_id,
                    provider_id,
                    &environment.revision,
                    true
                )
                .unwrap_err()
                .category,
                WslFailureCategory::Interrupted
            );
            runtime.artifacts.lock().unwrap().catalog = if missing {
                None
            } else {
                Some(b"external-change".to_vec())
            };
            let before = runtime.artifacts.lock().unwrap().clone();
            let recovery = WslApplication::with_runtime(store.clone(), runtime.clone());
            assert_eq!(
                recovery.recover_pending().unwrap_err().message_id,
                "wsl.recovery_conflict"
            );
            assert_eq!(runtime.artifacts.lock().unwrap().config, before.config);
            assert_eq!(runtime.artifacts.lock().unwrap().catalog, before.catalog);
            assert_eq!(runtime.active_locks.lock().unwrap().len(), 1);
            assert_eq!(
                Connection::open(store.paths().database())
                    .unwrap()
                    .query_row("SELECT count(*) FROM wsl_pending_operation", [], |row| row
                        .get::<_, i64>(
                        0
                    ))
                    .unwrap(),
                1
            );
        }
    }
}

#[test]
fn catalog_stopped_inventory_makes_no_guest_calls_and_clears_only_that_environment() {
    let stopped = probe();
    let mut second = stopped.clone();
    second.environment_id = "{99999999-9999-4999-8999-999999999999}".to_owned();
    second.display_name = "Other".to_owned();
    second.command_name = Some("Other".to_owned());
    let runtime = Arc::new(FakeRuntime::new(
        stopped.clone(),
        schema_v2_artifacts(
            &provider("22222222-2222-4222-8222-222222222222"),
            "shell-test",
        ),
    ));
    runtime.probes.lock().unwrap().push(second.clone());
    let (_temp, store, app) = application(runtime.clone());
    app.list().unwrap();
    let connection = Connection::open(store.paths().database()).unwrap();
    connection
        .execute("UPDATE wsl_environments SET pending_restart = 1", [])
        .unwrap();
    runtime.probes.lock().unwrap()[1].running = true;
    // A concurrent owner prevents reading the other environment but must not clear it.
    runtime.lock_busy.store(true, Ordering::SeqCst);
    let environments = app.list().unwrap();
    assert!(
        !environments
            .iter()
            .find(|env| env.environment_id == stopped.environment_id)
            .unwrap()
            .pending_restart
    );
    assert!(
        environments
            .iter()
            .find(|env| env.environment_id == second.environment_id)
            .unwrap()
            .pending_restart
    );
    assert_eq!(runtime.reads.load(Ordering::SeqCst), 0);
    assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);
    assert_eq!(runtime.writes.load(Ordering::SeqCst), 0);
    assert_eq!(runtime.terminations.load(Ordering::SeqCst), 0);
}
