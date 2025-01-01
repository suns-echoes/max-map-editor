use serde::{Serialize, Deserialize};
use std::sync::{Arc, Mutex};


#[derive(Debug, Serialize, Deserialize)]
pub struct AppState {
	pub max_path: String,
}


impl AppState {
	pub fn new() -> Self {
		AppState {
			max_path: String::from(""),
		}
	}

	pub fn set_max_path(&mut self, map: String) {
		self.max_path = map;
	}

	pub fn get_max_path(&self) -> String {
		self.max_path.clone()
	}
}


lazy_static::lazy_static! {
	pub static ref APP_STATE: Arc<Mutex<AppState>> = Arc::new(Mutex::new(AppState::new()));
}
