use lazy_static::lazy_static;
use std::sync::Mutex;


struct AppState {
	app_local_data_path: String,
	max_path: String,
}


lazy_static! {
	static ref APP_STATE: Mutex<AppState> = Mutex::new(AppState {
		app_data_path: String::from(""),
		app_local_data_path: String::from(""),
		max_path: String::from(""),
	});
}


pub fn set_app_local_data_path(path: String) {
	let mut app_state = APP_STATE.lock().unwrap();
	app_state.app_local_data_path = path;
}

pub fn get_app_local_data_path() -> String {
	let app_state = APP_STATE.lock().unwrap();
	app_state.app_local_data_path.clone()
}

pub fn set_max_path(path: String) {
	let mut app_state = APP_STATE.lock().unwrap();
	app_state.max_path = path;
}

pub fn get_max_path() -> String {
	let app_state = APP_STATE.lock().unwrap();
	app_state.max_path.clone()
}
