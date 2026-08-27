use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Emitter, Manager, Window, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tokio_util::sync::CancellationToken;

use crate::commands::{EnvironmentRuntime, IssueLogRuntime, ProviderRuntime, SessionRuntime};
use crate::diagnostics::IssueLogLevel;
use crate::environment::{AuthenticationMode, EnvironmentSnapshot, EnvironmentState};
use crate::provider::ProviderSummary;
use crate::state::StateStore;

const TRAY_ID: &str = "gpteasy";
const STATUS_ID: &str = "environment-status";
const SETTINGS_ID: &str = "settings";
const EXIT_ID: &str = "exit";
const PROVIDER_PREFIX: &str = "provider:";
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(30);

fn log_error(app: &AppHandle, event: &str, message_id: &str, details: &str) {
    app.state::<IssueLogRuntime>().store.append(
        IssueLogLevel::Error,
        event,
        message_id,
        Some(details.to_owned()),
    );
}

pub(crate) struct LifecycleRuntime {
    explicit_exit: AtomicBool,
    background_cancellation: CancellationToken,
    state_store: StateStore,
}

impl LifecycleRuntime {
    pub(crate) fn new(state_store: StateStore) -> Self {
        Self {
            explicit_exit: AtomicBool::new(false),
            background_cancellation: CancellationToken::new(),
            state_store,
        }
    }

    fn request_exit(&self) {
        self.explicit_exit.store(true, Ordering::SeqCst);
        self.background_cancellation.cancel();
    }

    fn should_hide_on_close(&self) -> bool {
        !self.explicit_exit.load(Ordering::SeqCst)
    }

    fn background_cancellation(&self) -> CancellationToken {
        self.background_cancellation.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayCommand {
    ShowSettings,
    Exit,
    SwitchProvider(String),
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayProviderAction {
    Ignore,
    OpenSwitchPlan,
    OpenSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayEffect {
    ShowSettings,
    OpenProviderSwitchPlan(String),
    Exit,
    None,
}

#[derive(Debug, PartialEq, Eq)]
struct TrayProviderItem {
    id: String,
    label: String,
    checked: bool,
    enabled: bool,
}

pub(crate) fn setup(app: &App) -> tauri::Result<()> {
    let menu = build_menu(app.app_handle(), &[], None)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("GPTEasy")
        .show_menu_on_left_click(true)
        .on_menu_event(handle_menu_event);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    refresh(app.app_handle())?;
    start_pending_observer(app.app_handle().clone());
    Ok(())
}

pub(crate) fn refresh(app: &AppHandle) -> tauri::Result<()> {
    let refresh_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let providers = match refresh_handle.state::<ProviderRuntime>().list() {
            Ok(providers) => providers,
            Err(failure) => {
                log_error(
                    &refresh_handle,
                    "tray.provider_list",
                    failure.message_id,
                    &format!("category={:?}", failure.category),
                );
                Vec::new()
            }
        };
        let snapshot = match refresh_handle.state::<EnvironmentRuntime>().inspect() {
            Ok(snapshot) => Some(snapshot),
            Err(failure) => {
                log_error(
                    &refresh_handle,
                    "tray.environment_inspect",
                    failure.message_id,
                    &format!("category={:?}", failure.category),
                );
                None
            }
        };
        install_menu(refresh_handle, providers, snapshot);
    });
    Ok(())
}

pub(crate) fn refresh_with_snapshot(
    app: &AppHandle,
    snapshot: &EnvironmentSnapshot,
) -> tauri::Result<()> {
    let refresh_handle = app.clone();
    let snapshot = snapshot.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let providers = match refresh_handle.state::<ProviderRuntime>().list() {
            Ok(providers) => providers,
            Err(failure) => {
                log_error(
                    &refresh_handle,
                    "tray.provider_list",
                    failure.message_id,
                    &format!("category={:?}", failure.category),
                );
                Vec::new()
            }
        };
        install_menu(refresh_handle, providers, Some(snapshot));
    });
    Ok(())
}

fn install_menu(
    app: AppHandle,
    providers: Vec<ProviderSummary>,
    snapshot: Option<EnvironmentSnapshot>,
) {
    let menu_handle = app.clone();
    if app
        .run_on_main_thread(move || {
            let Some(tray) = menu_handle.tray_by_id(TRAY_ID) else {
                log_error(
                    &menu_handle,
                    "tray.install_menu",
                    "tray.unavailable",
                    "category=tray",
                );
                return;
            };
            let menu = match build_menu(&menu_handle, &providers, snapshot.as_ref()) {
                Ok(menu) => menu,
                Err(_) => {
                    log_error(
                        &menu_handle,
                        "tray.build_menu",
                        "tray.menu_build_failed",
                        "category=tray",
                    );
                    return;
                }
            };
            if tray.set_menu(Some(menu)).is_err() {
                log_error(
                    &menu_handle,
                    "tray.install_menu",
                    "tray.menu_install_failed",
                    "category=tray",
                );
            }
        })
        .is_err()
    {
        log_error(
            &app,
            "tray.main_thread",
            "runtime.main_thread_unavailable",
            "category=runtime",
        );
    }
}

pub(crate) fn handle_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    let lifecycle = window.state::<LifecycleRuntime>();
    if !lifecycle.should_hide_on_close() {
        return;
    }
    api.prevent_close();
    if window.hide().is_err() {
        log_error(
            window.app_handle(),
            "window.hide",
            "window.hide_failed",
            "category=window",
        );
    }
    window.state::<SessionRuntime>().suspend();
    if lifecycle.state_store.should_show_first_close_notice() {
        let shown = window
            .notification()
            .builder()
            .title("GPTEasy 仍在运行")
            .body("可从系统托盘重新打开设置或退出 GPTEasy。")
            .show()
            .is_ok();
        if !shown {
            log_error(
                window.app_handle(),
                "window.close_notification",
                "notification.show_failed",
                "category=notification",
            );
        }
        if !mark_close_notice_after_display(&lifecycle.state_store, shown) {
            log_error(
                window.app_handle(),
                "window.close_notice_state",
                "state.write_failed",
                "category=state_unavailable",
            );
        }
    }
}

fn mark_close_notice_after_display(state_store: &StateStore, shown: bool) -> bool {
    if shown {
        state_store.mark_first_close_notice_seen()
    } else {
        true
    }
}

fn build_menu(
    app: &AppHandle,
    providers: &[ProviderSummary],
    snapshot: Option<&EnvironmentSnapshot>,
) -> tauri::Result<Menu<tauri::Wry>> {
    let status = MenuItemBuilder::with_id(STATUS_ID, status_text(snapshot))
        .enabled(false)
        .build(app)?;
    let settings = MenuItemBuilder::with_id(SETTINGS_ID, "设置...").build(app)?;
    let exit = MenuItemBuilder::with_id(EXIT_ID, "退出 GPTEasy").build(app)?;
    let mut menu = MenuBuilder::new(app).item(&status).separator();
    for provider in tray_provider_items(providers, snapshot) {
        let item = CheckMenuItemBuilder::with_id(provider.id, provider.label)
            .checked(provider.checked)
            .enabled(provider.enabled)
            .build(app)?;
        menu = menu.item(&item);
    }
    menu.separator().item(&settings).item(&exit).build()
}

fn tray_provider_items(
    providers: &[ProviderSummary],
    snapshot: Option<&EnvironmentSnapshot>,
) -> Vec<TrayProviderItem> {
    let current_provider_id = snapshot
        .and_then(|snapshot| snapshot.current_provider.as_ref())
        .map(|provider| provider.id.as_str());
    providers
        .iter()
        .map(|provider| TrayProviderItem {
            id: format!("{PROVIDER_PREFIX}{}", provider.id),
            label: escape_menu_text(&provider.name),
            checked: current_provider_id == Some(provider.id.as_str()),
            enabled: snapshot.is_some(),
        })
        .collect()
}

fn status_text(snapshot: Option<&EnvironmentSnapshot>) -> String {
    let Some(snapshot) = snapshot else {
        return "状态：无法读取".to_owned();
    };
    match (snapshot.state, snapshot.mode) {
        (EnvironmentState::Managed, Some(AuthenticationMode::Provider)) => snapshot
            .current_provider
            .as_ref()
            .map(|provider| format!("当前供应商：{}", provider.name))
            .unwrap_or_else(|| "状态：供应商模式".to_owned()),
        (EnvironmentState::Managed, Some(AuthenticationMode::OpenaiLogin)) => {
            "状态：OpenAI 登录".to_owned()
        }
        (EnvironmentState::External, _) => "状态：外部配置".to_owned(),
        (EnvironmentState::Conflict, _) => "状态：管理冲突".to_owned(),
        _ => "状态：无法读取".to_owned(),
    }
}

fn escape_menu_text(value: &str) -> String {
    value.replace('&', "&&")
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let command = parse_command(event.id().as_ref());
    if matches!(command, TrayCommand::SwitchProvider(_)) {
        plan_provider_switch(app, command);
        return;
    }
    execute_tray_effect(app, plan_tray_effect(command, None));
}

fn plan_provider_switch(app: &AppHandle, command: TrayCommand) {
    let inspect_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = match inspect_handle.state::<EnvironmentRuntime>().inspect() {
            Ok(snapshot) => Some(snapshot),
            Err(failure) => {
                log_error(
                    &inspect_handle,
                    "tray.provider_switch_inspect",
                    failure.message_id,
                    &format!("category={:?}", failure.category),
                );
                None
            }
        };
        let effect = plan_tray_effect(command, snapshot.as_ref());
        let effect_handle = inspect_handle.clone();
        if inspect_handle
            .run_on_main_thread(move || {
                execute_tray_effect(&effect_handle, effect);
            })
            .is_err()
        {
            log_error(
                &inspect_handle,
                "tray.main_thread",
                "runtime.main_thread_unavailable",
                "category=runtime",
            );
        }
    });
}

fn execute_tray_effect(app: &AppHandle, effect: TrayEffect) {
    match effect {
        TrayEffect::ShowSettings => show_settings(app),
        TrayEffect::OpenProviderSwitchPlan(provider_id) => {
            show_settings(app);
            if app.emit("provider-switch-requested", provider_id).is_err() {
                log_error(
                    app,
                    "tray.provider_switch_event",
                    "event.emit_failed",
                    "category=event",
                );
            }
        }
        TrayEffect::Exit => {
            app.state::<LifecycleRuntime>().request_exit();
            app.state::<ProviderRuntime>().shutdown_requests();
            app.state::<SessionRuntime>().shutdown();
            app.exit(0);
        }
        TrayEffect::None => {}
    }
}

fn parse_command(id: &str) -> TrayCommand {
    match id {
        SETTINGS_ID => TrayCommand::ShowSettings,
        EXIT_ID => TrayCommand::Exit,
        _ => id
            .strip_prefix(PROVIDER_PREFIX)
            .filter(|provider_id| !provider_id.is_empty())
            .map(|provider_id| TrayCommand::SwitchProvider(provider_id.to_owned()))
            .unwrap_or(TrayCommand::Ignore),
    }
}

fn plan_provider_action(snapshot: &EnvironmentSnapshot, provider_id: &str) -> TrayProviderAction {
    if snapshot
        .current_provider
        .as_ref()
        .map(|provider| provider.id.as_str())
        == Some(provider_id)
    {
        return TrayProviderAction::Ignore;
    }
    match snapshot.state {
        EnvironmentState::External => TrayProviderAction::OpenSwitchPlan,
        EnvironmentState::Conflict if snapshot.takeover_available => {
            TrayProviderAction::OpenSwitchPlan
        }
        EnvironmentState::Conflict => TrayProviderAction::OpenSettings,
        EnvironmentState::Managed => TrayProviderAction::OpenSwitchPlan,
    }
}

fn plan_tray_effect(command: TrayCommand, snapshot: Option<&EnvironmentSnapshot>) -> TrayEffect {
    match command {
        TrayCommand::ShowSettings => TrayEffect::ShowSettings,
        TrayCommand::Exit => TrayEffect::Exit,
        TrayCommand::Ignore => TrayEffect::None,
        TrayCommand::SwitchProvider(provider_id) => {
            let Some(snapshot) = snapshot else {
                return TrayEffect::ShowSettings;
            };
            match plan_provider_action(snapshot, &provider_id) {
                TrayProviderAction::Ignore => TrayEffect::None,
                TrayProviderAction::OpenSettings => TrayEffect::ShowSettings,
                TrayProviderAction::OpenSwitchPlan => {
                    TrayEffect::OpenProviderSwitchPlan(provider_id)
                }
            }
        }
    }
}

pub(crate) fn show_settings(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        log_error(
            app,
            "window.show_settings",
            "window.unavailable",
            "category=window",
        );
        return;
    };
    app.state::<SessionRuntime>().resume();
    if window.show().is_err() {
        log_error(
            app,
            "window.show_settings",
            "window.show_failed",
            "category=window",
        );
    }
    if window.unminimize().is_err() {
        log_error(
            app,
            "window.unminimize",
            "window.unminimize_failed",
            "category=window",
        );
    }
    if window.set_focus().is_err() {
        log_error(
            app,
            "window.focus",
            "window.focus_failed",
            "category=window",
        );
    }

    let inspect_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = match inspect_handle.state::<EnvironmentRuntime>().inspect() {
            Ok(snapshot) => snapshot,
            Err(failure) => {
                log_error(
                    &inspect_handle,
                    "window.environment_inspect",
                    failure.message_id,
                    &format!("category={:?}", failure.category),
                );
                return;
            }
        };
        let refresh_handle = inspect_handle.clone();
        if inspect_handle
            .run_on_main_thread(move || {
                if refresh_with_snapshot(&refresh_handle, &snapshot).is_err() {
                    log_error(
                        &refresh_handle,
                        "tray.refresh",
                        "tray.refresh_failed",
                        "category=tray",
                    );
                }
            })
            .is_err()
        {
            log_error(
                &inspect_handle,
                "tray.main_thread",
                "runtime.main_thread_unavailable",
                "category=runtime",
            );
        }
    });
}

fn start_pending_observer(app: AppHandle) {
    let cancellation = app.state::<LifecycleRuntime>().background_cancellation();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(OBSERVATION_INTERVAL);
        tokio::select! {
            _ = cancellation.cancelled() => return,
            _ = interval.tick() => {}
        }
        let mut last_failure = None;
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => break,
                _ = interval.tick() => {}
            }
            let inspect_handle = app.clone();
            let outcome = tauri::async_runtime::spawn_blocking(move || {
                let runtime = inspect_handle.state::<EnvironmentRuntime>();
                match runtime.has_pending_restart() {
                    Ok(false) => Ok(None),
                    Ok(true) => runtime.inspect().map(Some).map_err(|failure| {
                        (
                            failure.message_id,
                            format!("category={:?}", failure.category),
                        )
                    }),
                    Err(failure) => Err((
                        failure.message_id,
                        format!("category={:?}", failure.category),
                    )),
                }
            })
            .await;
            if cancellation.is_cancelled() {
                break;
            }
            let snapshot = match outcome {
                Ok(Ok(snapshot)) => {
                    last_failure = None;
                    snapshot
                }
                Ok(Err((message_id, details))) => {
                    if last_failure != Some(message_id) {
                        log_error(&app, "tray.pending_observer", message_id, &details);
                        last_failure = Some(message_id);
                    }
                    None
                }
                Err(_) => {
                    let message_id = "runtime.background_task_failed";
                    if last_failure != Some(message_id) {
                        log_error(
                            &app,
                            "tray.pending_observer",
                            message_id,
                            "category=runtime",
                        );
                        last_failure = Some(message_id);
                    }
                    None
                }
            };
            if let Some(snapshot) = snapshot {
                if refresh_with_snapshot(&app, &snapshot).is_err() {
                    log_error(&app, "tray.refresh", "tray.refresh_failed", "category=tray");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::LoginStatus;
    use crate::consumer::ConsumerStatus;
    use crate::environment::{ConsumerStatuses, RestoreAvailability};
    use crate::state::{StatePaths, StateStore};
    use tempfile::TempDir;

    fn snapshot(state: EnvironmentState, mode: Option<AuthenticationMode>) -> EnvironmentSnapshot {
        EnvironmentSnapshot {
            state,
            mode,
            message_id: "test",
            revision: "revision".to_owned(),
            requires_takeover_confirmation: state != EnvironmentState::Managed,
            takeover_available: true,
            impacts: Vec::new(),
            current_provider: None,
            restore_availability: RestoreAvailability::NoBackup,
            restore_preview: None,
            login_status: LoginStatus::Unavailable,
            pending_restart: false,
            requires_consumer_confirmation: false,
            consumers: ConsumerStatuses {
                desktop: ConsumerStatus::Stopped,
                cli: ConsumerStatus::Stopped,
            },
        }
    }

    fn provider(id: &str, name: &str, recommendation_id: Option<&str>) -> ProviderSummary {
        ProviderSummary {
            id: id.to_owned(),
            name: name.to_owned(),
            base_url: format!("https://{id}.example/v1"),
            default_model: "model-a".to_owned(),
            verified_at_epoch_seconds: 1,
            is_current: false,
            recommendation_id: recommendation_id.map(str::to_owned),
            has_recommendation_update: false,
            recommendation_template_base_url: recommendation_id
                .map(|_| crate::provider::DAYWAY_BASE_URL.to_owned()),
        }
    }

    #[test]
    fn tray_projects_only_persisted_catalog_entries_in_catalog_order() {
        let saved = vec![
            provider("dayway-id", "DayWay", Some("dayway")),
            provider("ordinary-id", "Ordinary", None),
        ];
        let inspected = snapshot(
            EnvironmentState::Managed,
            Some(AuthenticationMode::Provider),
        );
        assert_eq!(
            tray_provider_items(&saved, Some(&inspected)),
            vec![
                TrayProviderItem {
                    id: "provider:dayway-id".to_owned(),
                    label: "DayWay".to_owned(),
                    checked: false,
                    enabled: true,
                },
                TrayProviderItem {
                    id: "provider:ordinary-id".to_owned(),
                    label: "Ordinary".to_owned(),
                    checked: false,
                    enabled: true,
                },
            ]
        );
        assert!(tray_provider_items(&[], Some(&inspected)).is_empty());
    }

    #[test]
    fn tray_keeps_the_current_provider_enabled_and_marks_it_checked() {
        let current = provider("current-id", "Current", None);
        let mut inspected = snapshot(
            EnvironmentState::Managed,
            Some(AuthenticationMode::Provider),
        );
        inspected.current_provider = Some(current.clone());

        assert_eq!(
            tray_provider_items(std::slice::from_ref(&current), Some(&inspected)),
            vec![TrayProviderItem {
                id: "provider:current-id".to_owned(),
                label: "Current".to_owned(),
                checked: true,
                enabled: true,
            }]
        );
        assert_eq!(
            tray_provider_items(&[current], None),
            vec![TrayProviderItem {
                id: "provider:current-id".to_owned(),
                label: "Current".to_owned(),
                checked: false,
                enabled: false,
            }]
        );
    }

    #[test]
    fn tray_does_not_mark_recorded_current_provider_for_external_openai_login() {
        let mut recorded_current = provider("provider-id", "Recorded", None);
        recorded_current.is_current = true;
        let inspected = snapshot(
            EnvironmentState::External,
            Some(AuthenticationMode::OpenaiLogin),
        );

        assert_eq!(
            tray_provider_items(&[recorded_current], Some(&inspected)),
            vec![TrayProviderItem {
                id: "provider:provider-id".to_owned(),
                label: "Recorded".to_owned(),
                checked: false,
                enabled: true,
            }]
        );
        assert_eq!(status_text(Some(&inspected)), "状态：外部配置");
    }

    #[test]
    fn tray_status_keeps_showing_the_current_environment_while_restart_is_pending() {
        let mut managed = snapshot(
            EnvironmentState::Managed,
            Some(AuthenticationMode::Provider),
        );
        managed.current_provider = Some(provider("current-id", "Current", None));
        managed.pending_restart = true;

        assert_eq!(status_text(Some(&managed)), "当前供应商：Current");
    }

    #[test]
    fn tray_commands_are_restricted_to_settings_exit_and_provider_selection() {
        assert_eq!(parse_command("settings"), TrayCommand::ShowSettings);
        assert_eq!(parse_command("exit"), TrayCommand::Exit);
        assert_eq!(
            parse_command("provider:provider-id"),
            TrayCommand::SwitchProvider("provider-id".to_owned())
        );
        assert_eq!(parse_command("restart-codex"), TrayCommand::Ignore);
    }

    #[test]
    fn tray_commands_plan_settings_exit_and_provider_confirmation_effects() {
        let managed = snapshot(
            EnvironmentState::Managed,
            Some(AuthenticationMode::Provider),
        );
        assert_eq!(
            plan_tray_effect(TrayCommand::ShowSettings, None),
            TrayEffect::ShowSettings
        );
        assert_eq!(plan_tray_effect(TrayCommand::Exit, None), TrayEffect::Exit);
        assert_eq!(
            plan_tray_effect(
                TrayCommand::SwitchProvider("provider-id".to_owned()),
                Some(&managed),
            ),
            TrayEffect::OpenProviderSwitchPlan("provider-id".to_owned())
        );
        assert_eq!(
            plan_tray_effect(TrayCommand::SwitchProvider("provider-id".to_owned()), None,),
            TrayEffect::ShowSettings
        );
    }

    #[test]
    fn tray_provider_selection_uses_confirmation_and_conflicts_open_settings() {
        let mut normal = snapshot(
            EnvironmentState::Managed,
            Some(AuthenticationMode::Provider),
        );
        normal.current_provider = Some(provider("current-id", "Current", None));
        assert_eq!(
            plan_tray_effect(
                TrayCommand::SwitchProvider("current-id".to_owned()),
                Some(&normal),
            ),
            TrayEffect::None
        );
        assert_eq!(
            plan_provider_action(&normal, "provider-id"),
            TrayProviderAction::OpenSwitchPlan
        );

        let mut running = normal.clone();
        running.requires_consumer_confirmation = true;
        assert_eq!(
            plan_provider_action(&running, "provider-id"),
            TrayProviderAction::OpenSwitchPlan
        );

        let external = snapshot(EnvironmentState::External, None);
        assert_eq!(
            plan_provider_action(&external, "provider-id"),
            TrayProviderAction::OpenSwitchPlan
        );

        let mut unsafe_conflict = snapshot(EnvironmentState::Conflict, None);
        unsafe_conflict.takeover_available = false;
        assert_eq!(
            plan_provider_action(&unsafe_conflict, "provider-id"),
            TrayProviderAction::OpenSettings
        );
    }

    #[test]
    fn failed_close_notice_display_keeps_the_notice_available_for_retry() {
        let temp = TempDir::new().expect("temp directory");
        let store = StateStore::new(StatePaths::from_root(temp.path()));
        assert!(store.bootstrap().is_ready());

        mark_close_notice_after_display(&store, false);
        assert!(store.should_show_first_close_notice());
        mark_close_notice_after_display(&store, true);
        assert!(!store.should_show_first_close_notice());
    }

    #[test]
    fn explicit_exit_disables_close_to_tray_interception() {
        let temp = TempDir::new().expect("temp directory");
        let lifecycle = LifecycleRuntime::new(StateStore::new(StatePaths::from_root(temp.path())));

        assert!(lifecycle.should_hide_on_close());
        assert!(!lifecycle.background_cancellation().is_cancelled());
        lifecycle.request_exit();
        assert!(!lifecycle.should_hide_on_close());
        assert!(lifecycle.background_cancellation().is_cancelled());
    }
}
