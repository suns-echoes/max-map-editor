use std::{
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

// -----------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct AppState {
    app_local_data_path: PathBuf,
    resource_path: PathBuf,
    max_path: PathBuf,
}

static APP_STATE: OnceLock<RwLock<AppState>> = OnceLock::new();

fn app_state() -> &'static RwLock<AppState> {
    APP_STATE.get_or_init(|| RwLock::new(AppState::default()))
}

// -----------------------------------------------------------------------------

fn read_state<R>(f: impl FnOnce(&AppState) -> R) -> R {
    let guard = app_state()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&guard)
}

fn write_state<R>(f: impl FnOnce(&mut AppState) -> R) -> R {
    let mut guard = app_state()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

// -----------------------------------------------------------------------------

pub fn get_app_local_data_path() -> PathBuf {
    read_state(|state| state.app_local_data_path.clone())
}

pub fn get_app_local_data_path_to(sub_path: impl AsRef<Path>) -> PathBuf {
	read_state(|state| state.app_local_data_path.join(sub_path))
}

pub fn get_app_local_data_path_as_str() -> String {
	read_state(|state| state.app_local_data_path.to_string_lossy().to_string())
}

pub fn set_app_local_data_path(path: impl AsRef<Path>) {
    write_state(|state| state.app_local_data_path = path.as_ref().to_path_buf());
}

// -----------------------------------------------------------------------------

pub fn get_resource_path() -> PathBuf {
    read_state(|state| state.resource_path.clone())
}

pub fn get_resource_path_to(sub_path: impl AsRef<Path>) -> PathBuf {
    read_state(|state| state.resource_path.join(sub_path))
}

pub fn get_resource_path_as_str() -> String {
	read_state(|state| state.resource_path.to_string_lossy().to_string())
}

pub fn set_resource_path(path: impl AsRef<Path>) {
    write_state(|state| state.resource_path = path.as_ref().to_path_buf());
}

// -----------------------------------------------------------------------------

pub fn get_max_path() -> PathBuf {
    read_state(|state| state.max_path.clone())
}

pub fn get_max_path_to(sub_path: impl AsRef<Path>) -> PathBuf {
	read_state(|state| state.max_path.join(sub_path))
}

pub fn get_max_path_as_str() -> String {
	read_state(|state| state.max_path.to_string_lossy().to_string())
}

pub fn set_max_path(path: impl AsRef<Path>) {
    write_state(|state| state.max_path = path.as_ref().to_path_buf());
}
