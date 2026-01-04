use time;
use std::{fs::OpenOptions, io::Write, path::{Path, PathBuf}, sync::{OnceLock, RwLock}};

// =============================================================================

static LOGGER_STATE: OnceLock<RwLock<LoggerState>> = OnceLock::new();

#[derive(Default)]
struct LoggerState {
	file_path: Option<PathBuf>,
	file_logging_disabled: bool,
}

fn state() -> &'static RwLock<LoggerState> {
	LOGGER_STATE.get_or_init(|| RwLock::new(LoggerState::default()))
}

fn enable_file_logging(path: PathBuf) {
	if let Ok(mut guard) = state().write() {
		guard.file_path = Some(path);
		guard.file_logging_disabled = false;
	}
}

fn disable_file_logging(reason: &str) {
	if let Ok(mut guard) = state().write() {
		if guard.file_logging_disabled {
			return;
		}
		xlog_error(reason);
		xlog_warn("File logger disabled; future log entries will go to console only.");
		guard.file_path = None;
		guard.file_logging_disabled = true;
	}
}

fn get_log_file_path() -> Option<PathBuf> {
	match state().read() {
		Ok(guard) if !guard.file_logging_disabled => guard.file_path.clone(),
		Ok(_) => None,
		Err(_) => {
			disable_file_logging("Logger state poisoned; disabling file logging.");
			None
		}
	}
}

// =============================================================================

fn create_log_file(path: &Path) -> Result<(), String> {
	OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(true)
		.open(path)
		.map(|_| ())
		.map_err(|e| format!("Unable to create log file {}: {}", path.display(), e))
}

fn append_log_file(message: &str) {
	let Some(log_file_path) = get_log_file_path() else {
		return;
	};

	let result = OpenOptions::new()
		.create(true)
		.append(true)
		.open(&log_file_path)
		.and_then(|mut file| writeln!(file, "{}", message));

	if let Err(e) = result {
		disable_file_logging(&format!(
			"Unable to append to log file {}: {}",
			log_file_path.display(),
			e
		));
	}
}

// =============================================================================

fn get_yyyymmdd_hhmmss_time() -> String {
	let now = time::OffsetDateTime::now_utc();

	format!(
		"{}-{}-{} {}:{}:{}",
		now.year(),
		format!("{:02}", now.month() as u8),
		format!("{:02}", now.day()),
		format!("{:02}", now.hour()),
		format!("{:02}", now.minute()),
		format!("{:02}", now.second())
	)
}

pub fn xlog_message(level: &str, console_log: fn(&str), message: &str) {
    let formatted_message = {
        let timestamp = get_yyyymmdd_hhmmss_time();
        format!("{} {}: {}", timestamp, level, message)
    };
    console_log(&formatted_message);
    append_log_file(&formatted_message);
}

pub fn xlog_success(message: &str) {
	eprintln!("\x1b[32m{}\x1b[0m", message);
}

pub fn xlog_info(message: &str) {
	eprintln!("\x1b[34m{}\x1b[0m", message);
}

pub fn xlog_warn(message: &str) {
	eprintln!("\x1b[33m{}\x1b[0m", message);
}

pub fn xlog_error(message: &str) {
	eprintln!("\x1b[31m{}\x1b[0m", message);
}

// =============================================================================

pub fn xlog_init(log_dir: &Path, log_file_name: &str) {
	let log_path = PathBuf::from(log_dir).join(log_file_name);

	if let Err(err) = create_log_file(&log_path) {
		disable_file_logging(&format!("logger::init: {}", err));
		return;
	}

	enable_file_logging(log_path);
}

// =============================================================================

#[macro_export]
macro_rules! xlog_success {
	($($arg:tt)*) => {{
		$crate::xlog_message("SUCCESS", $crate::xlog_success, &format!($($arg)*));
	}};
}

#[macro_export]
macro_rules! xlog_info {
	($($arg:tt)*) => {{
		$crate::xlog_message("INFO", $crate::xlog_info, &format!($($arg)*));
	}};
}

#[macro_export]
macro_rules! xlog_warn {
	($($arg:tt)*) => {{
		$crate::xlog_message("WARNING", $crate::xlog_warn, &format!($($arg)*));
	}};
}

#[macro_export]
macro_rules! xlog_error {
	($($arg:tt)*) => {{
		$crate::xlog_message("ERROR", $crate::xlog_error, &format!($($arg)*));
	}};
}
