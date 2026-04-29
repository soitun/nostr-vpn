use crate::tray_runtime::refresh_tray_menu;

use super::*;

pub(crate) fn with_backend<T>(
    state: State<'_, AppState>,
    f: impl FnOnce(&mut NvpnBackend) -> Result<T>,
) -> Result<T, String> {
    let mut backend = state
        .backend
        .lock()
        .map_err(|_| "backend lock poisoned".to_string())?;
    f(&mut backend).map_err(|error| error.to_string())
}

pub(crate) async fn run_blocking_mutex_action<S, T, F>(
    state: Arc<Mutex<S>>,
    state_name: &'static str,
    action: F,
) -> Result<T, String>
where
    S: Send + 'static,
    T: Send + 'static,
    F: FnOnce(&mut S) -> Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut state = state
            .lock()
            .map_err(|_| format!("{state_name} lock poisoned"))?;
        action(&mut state).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("failed to join {state_name} action task: {error}"))?
}

pub(crate) async fn with_backend_async<T, F>(
    state: State<'_, AppState>,
    action: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut NvpnBackend) -> Result<T> + Send + 'static,
{
    run_blocking_mutex_action(state.backend.clone(), "backend", action).await
}

pub(crate) fn apply_windows_subprocess_flags(command: &mut ProcessCommand) -> &mut ProcessCommand {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[tauri::command]
pub(crate) async fn get_state(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, |backend| {
        backend.refresh_runtime_state();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn tick(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, |backend| {
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn connect_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, |backend| {
        backend.connect_session()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn disconnect_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, |backend| {
        backend.disconnect_session()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn install_cli(state: State<'_, AppState>) -> Result<UiState, String> {
    with_backend_async(state, |backend| {
        backend.install_cli_binary()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await
}

#[tauri::command]
pub(crate) async fn uninstall_cli(state: State<'_, AppState>) -> Result<UiState, String> {
    with_backend_async(state, |backend| {
        backend.uninstall_cli_binary()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await
}

#[tauri::command]
pub(crate) async fn install_system_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, |backend| {
        backend.install_system_service()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn uninstall_system_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, |backend| {
        backend.uninstall_system_service()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn enable_system_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, |backend| {
        backend.enable_system_service()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn disable_system_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, |backend| {
        backend.disable_system_service()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn add_network(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.add_network(&name)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn rename_network(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    name: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.rename_network(&network_id, &name)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn remove_network(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.remove_network(&network_id)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn set_network_mesh_id(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    mesh_id: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.set_network_mesh_id(&network_id, &mesh_id)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn set_network_enabled(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    enabled: bool,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.set_network_enabled(&network_id, enabled)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn set_network_join_requests_enabled(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    enabled: bool,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.set_network_join_requests_enabled(&network_id, enabled)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn request_network_join(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.request_network_join(&network_id)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn add_participant(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    npub: String,
    alias: Option<String>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.add_participant(&network_id, &npub, alias.as_deref())?;
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn add_admin(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    npub: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.add_admin(&network_id, &npub)?;
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn import_network_invite(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    invite: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.import_network_invite(&invite)?;
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn start_lan_pairing(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.start_lan_pairing()?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn stop_lan_pairing(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.stop_lan_pairing();
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn remove_participant(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    npub: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.remove_participant(&network_id, &npub)?;
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn remove_admin(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    npub: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.remove_admin(&network_id, &npub)?;
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn accept_join_request(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    network_id: String,
    requester_npub: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.accept_join_request(&network_id, &requester_npub)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn set_participant_alias(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    npub: String,
    alias: String,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.set_participant_alias(&npub, &alias)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}

#[tauri::command]
pub(crate) async fn add_relay(
    state: State<'_, AppState>,
    relay: String,
) -> Result<UiState, String> {
    with_backend_async(state, move |backend| {
        backend.add_relay(&relay)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await
}

#[tauri::command]
pub(crate) async fn remove_relay(
    state: State<'_, AppState>,
    relay: String,
) -> Result<UiState, String> {
    with_backend_async(state, move |backend| {
        backend.remove_relay(&relay)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await
}

#[tauri::command]
pub(crate) async fn update_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> Result<UiState, String> {
    let ui = with_backend_async(state, move |backend| {
        backend.update_settings(patch)?;
        backend.tick();
        Ok(backend.ui_state())
    })
    .await?;
    refresh_tray_menu(&app);
    Ok(ui)
}
