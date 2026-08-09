use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager, Window, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use crate::commands::{EnvironmentRuntime, ProviderRuntime};
use crate::environment::{AuthenticationMode, EnvironmentSnapshot, EnvironmentState};
use crate::provider::ProviderSummary;
use crate::state::StateStore;

const TRAY_ID: &str = "gpteasy";
const STATUS_ID: &str = "environment-status";
const SETTINGS_ID: &str = "settings";
const EXIT_ID: &str = "exit";
const PROVIDER_PREFIX: &str = "provider:";
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct LifecycleRuntime {
    explicit_exit: AtomicBool,
    state_store: StateStore,
}

impl LifecycleRuntime {
    pub(crate) fn new(state_store: StateStore) -> Self {
        Self {
            explicit_exit: AtomicBool::new(false),
            state_store,
        }
    }

    fn request_exit(&self) {
        self.explicit_exit.store(true, Ordering::SeqCst);
    }

    fn should_hide_on_close(&self) -> bool {
        !self.explicit_exit.load(Ordering::SeqCst)
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
    ConfirmAndSwitch,
    OpenSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayEffect {
    ShowSettings,
    ConfirmProviderSwitch {
        provider_id: String,
        expected_revision: String,
    },
    Exit,
    None,
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
        let providers = refresh_handle
            .state::<ProviderRuntime>()
            .list()
            .unwrap_or_default();
        let snapshot = refresh_handle.state::<EnvironmentRuntime>().inspect().ok();
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
        let providers = refresh_handle
            .state::<ProviderRuntime>()
            .list()
            .unwrap_or_default();
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
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = menu_handle.tray_by_id(TRAY_ID) else {
            return;
        };
        let Ok(menu) = build_menu(&menu_handle, &providers, snapshot.as_ref()) else {
            return;
        };
        let _ = tray.set_menu(Some(menu));
    });
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
    let _ = window.hide();
    if lifecycle.state_store.should_show_first_close_notice() {
        let shown = window
            .notification()
            .builder()
            .title("GPTEasy 仍在运行")
            .body("可从系统托盘重新打开设置或退出 GPTEasy。")
            .show()
            .is_ok();
        mark_close_notice_after_display(&lifecycle.state_store, shown);
    }
}

fn mark_close_notice_after_display(state_store: &StateStore, shown: bool) {
    if shown {
        let _ = state_store.mark_first_close_notice_seen();
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
    for provider in providers {
        let enabled = snapshot.is_some() && !provider.is_current;
        let item = CheckMenuItemBuilder::with_id(
            format!("{PROVIDER_PREFIX}{}", provider.id),
            escape_menu_text(&provider.name),
        )
        .checked(provider.is_current)
        .enabled(enabled)
        .build(app)?;
        menu = menu.item(&item);
    }
    menu.separator().item(&settings).item(&exit).build()
}

fn status_text(snapshot: Option<&EnvironmentSnapshot>) -> String {
    let Some(snapshot) = snapshot else {
        return "状态：无法读取".to_owned();
    };
    if snapshot.pending_restart {
        return "状态：待重启".to_owned();
    }
    match (snapshot.state, snapshot.mode) {
        (EnvironmentState::Managed, Some(AuthenticationMode::Provider)) => snapshot
            .current_provider
            .as_ref()
            .map(|provider| format!("当前供应商：{}", provider.name))
            .unwrap_or_else(|| "状态：供应商模式".to_owned()),
        (EnvironmentState::Managed, Some(AuthenticationMode::OpenaiLogin)) => {
            "状态：OpenAI 登录模式".to_owned()
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
        let snapshot = inspect_handle.state::<EnvironmentRuntime>().inspect().ok();
        let effect = plan_tray_effect(command, snapshot.as_ref());
        let effect_handle = inspect_handle.clone();
        let _ = inspect_handle.run_on_main_thread(move || {
            execute_tray_effect(&effect_handle, effect);
        });
    });
}

fn execute_tray_effect(app: &AppHandle, effect: TrayEffect) {
    match effect {
        TrayEffect::ShowSettings => show_settings(app),
        TrayEffect::ConfirmProviderSwitch {
            provider_id,
            expected_revision,
        } => confirm_provider_switch(app, &provider_id, &expected_revision),
        TrayEffect::Exit => {
            app.state::<LifecycleRuntime>().request_exit();
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
        EnvironmentState::External | EnvironmentState::Conflict => TrayProviderAction::OpenSettings,
        EnvironmentState::Managed => TrayProviderAction::ConfirmAndSwitch,
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
                TrayProviderAction::ConfirmAndSwitch => TrayEffect::ConfirmProviderSwitch {
                    provider_id,
                    expected_revision: snapshot.revision.clone(),
                },
            }
        }
    }
}

fn confirm_provider_switch(app: &AppHandle, provider_id: &str, expected_revision: &str) {
    if !confirm_switch_risk() {
        return;
    }
    let switch_handle = app.clone();
    let provider_id = provider_id.to_owned();
    let expected_revision = expected_revision.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let result = switch_handle
            .state::<EnvironmentRuntime>()
            .switch_provider(&provider_id, &expected_revision);
        let result_handle = switch_handle.clone();
        let _ = switch_handle.run_on_main_thread(move || match result {
            Ok(snapshot) => {
                let _ = refresh_with_snapshot(&result_handle, &snapshot);
            }
            Err(_) => show_settings(&result_handle),
        });
    });
}

#[cfg(windows)]
fn confirm_switch_risk() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDOK, MB_ICONWARNING, MB_OKCANCEL, MessageBoxW,
    };

    let message = "切换后，切换前已运行或可能遗漏的 Codex 消费者需要退出后才能读取新配置。GPTEasy 将保守标记为待重启。是否继续？"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let title = "切换供应商"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let response = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OKCANCEL | MB_ICONWARNING,
        )
    };
    response == IDOK
}

#[cfg(not(windows))]
fn confirm_switch_risk() -> bool {
    false
}

fn show_settings(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();

    let inspect_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(snapshot) = inspect_handle.state::<EnvironmentRuntime>().inspect() else {
            return;
        };
        let refresh_handle = inspect_handle.clone();
        let _ = inspect_handle.run_on_main_thread(move || {
            let _ = refresh_with_snapshot(&refresh_handle, &snapshot);
        });
    });
}

fn start_pending_observer(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(OBSERVATION_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            let inspect_handle = app.clone();
            let snapshot = tauri::async_runtime::spawn_blocking(move || {
                let runtime = inspect_handle.state::<EnvironmentRuntime>();
                runtime
                    .has_pending_restart()
                    .ok()
                    .filter(|pending| *pending)
                    .and_then(|_| runtime.inspect().ok())
            })
            .await
            .ok()
            .flatten();
            if let Some(snapshot) = snapshot {
                let _ = refresh_with_snapshot(&app, &snapshot);
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
            impacts: Vec::new(),
            current_provider: None,
            restore_availability: RestoreAvailability::NoBackup,
            login_status: LoginStatus::Unavailable,
            pending_restart: false,
            requires_consumer_confirmation: false,
            consumers: ConsumerStatuses {
                desktop: ConsumerStatus::Stopped,
                cli: ConsumerStatus::Stopped,
            },
        }
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
            TrayEffect::ConfirmProviderSwitch {
                provider_id: "provider-id".to_owned(),
                expected_revision: "revision".to_owned(),
            }
        );
        assert_eq!(
            plan_tray_effect(TrayCommand::SwitchProvider("provider-id".to_owned()), None,),
            TrayEffect::ShowSettings
        );
    }

    #[test]
    fn tray_provider_selection_uses_confirmation_and_conflicts_open_settings() {
        let normal = snapshot(
            EnvironmentState::Managed,
            Some(AuthenticationMode::Provider),
        );
        assert_eq!(
            plan_provider_action(&normal, "provider-id"),
            TrayProviderAction::ConfirmAndSwitch
        );

        let mut running = normal.clone();
        running.requires_consumer_confirmation = true;
        assert_eq!(
            plan_provider_action(&running, "provider-id"),
            TrayProviderAction::ConfirmAndSwitch
        );

        let external = snapshot(EnvironmentState::External, None);
        assert_eq!(
            plan_provider_action(&external, "provider-id"),
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
        lifecycle.request_exit();
        assert!(!lifecycle.should_hide_on_close());
    }
}
