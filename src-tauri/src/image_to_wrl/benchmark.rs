use std::io::Write;

use crate::image_to_wrl::log;

pub fn get_start_time() -> std::time::Instant {
    std::time::Instant::now()
}

pub fn get_elapsed_time(start: std::time::Instant) -> std::time::Duration {
    start.elapsed()
}

pub fn print_elapsed_time(start: std::time::Instant) {
    let duration = get_elapsed_time(start);
    log::info(&format!("Execution time: {:.2?}", duration));
}

pub fn measure_time<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let start = get_start_time();
    let result = f();
    let duration = start.elapsed();
    log::info(&format!("Execution time: {:.2?}", duration));
    std::io::stdout().flush().unwrap();
    result
}
