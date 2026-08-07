use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use crate::startup::{StartupCoordinator, StartupSnapshot};

pub(crate) struct StartupRuntime {
    coordinator: Mutex<StartupCoordinator>,
}

impl StartupRuntime {
    pub(crate) fn new(coordinator: StartupCoordinator) -> Self {
        Self {
            coordinator: Mutex::new(coordinator),
        }
    }

    fn inspect(&self) -> Result<StartupSnapshot, CommandFailure> {
        self.coordinator
            .lock()
            .map(|coordinator| coordinator.inspect())
            .map_err(|_| CommandFailure {
                message_id: "startup.internal_state_unavailable",
            })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandFailure {
    message_id: &'static str,
}

#[tauri::command]
pub(crate) fn get_startup_snapshot(
    state: State<'_, StartupRuntime>,
) -> Result<StartupSnapshot, CommandFailure> {
    state.inspect()
}

#[tauri::command]
pub(crate) fn refresh_startup_snapshot(
    state: State<'_, StartupRuntime>,
) -> Result<StartupSnapshot, CommandFailure> {
    state.inspect()
}
