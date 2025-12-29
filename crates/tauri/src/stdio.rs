pub fn error(message: &str) {
	eprintln!("\x1b[31m>> {}\x1b[0m", message);
}

pub fn info(message: &str) {
	eprintln!("\x1b[34m>> {}\x1b[0m", message);
}

pub fn warn(message: &str) {
	eprintln!("\x1b[33m>> {}\x1b[0m", message);
}

pub fn success(message: &str) {
	eprintln!("\x1b[32m>> {}\x1b[0m", message);
}

pub fn message(message: &str) {
	eprintln!("{}", message);
}
