use std::io::Write;

pub fn ok(message: &str, execution_time: Option<std::time::Duration>) {
    if let Some(time) = execution_time {
        println!(
            "\x1b[1;32m✔\x1b[0m \x1b[32m{} ({:.2?})\x1b[0m",
            message, time
        );
    } else {
        println!("\x1b[1;32m✔\x1b[0m \x1b[32m{}\x1b[0m", message);
    }
}

pub fn error(message: &str) {
    eprintln!("\x1b[1;31m✘\x1b[0m \x1b[31m{}\x1b[0m", message);
}

pub fn warning(message: &str) {
    eprintln!("\x1b[1;33m⚠\x1b[0m \x1b[33m{}\x1b[0m", message);
}

pub fn info(message: &str) {
    println!("\x1b[1;34mℹ\x1b[0m \x1b[34m{}\x1b[0m", message);
}

pub fn action(message: &str) {
    println!("\x1b[1;35m➤\x1b[0m \x1b[35m{}\x1b[0m", message);
}

pub fn param(name: &str, value: &str) {
    println!("\x1b[36m🮶 {}:\x1b[0m \x1b[1;36m{}\x1b[0m", name, value);
}

pub fn title(title: &str) {
    println!("\n\x1b[1;33m{}\x1b[0m", title);
}

pub fn nl() {
    println!();
}

pub fn line_up() {
    print!("\x1b[A\r\x1b[J");
}

pub fn progress(current: usize, total: usize) {
    let percentage = (current as f64 / total as f64) * 100.0;
    print!(
        "\r\x1b[1;36mProgress: [{:<50}] {:.2}%\x1b[0m",
        "=".repeat((current * 50) / total),
        percentage
    );
    std::io::stdout().flush().unwrap();
}

pub fn start_progress() {
    nl();
    progress(0, 1);
}

pub fn color_show() {
    // Foreground colors
    println!("Foreground colors:");
    for code in 30..=37 {
        print!("\x1b[{}m {:>3} \x1b[0m", code, code);
    }
    println!();

    // Bright foreground colors
    println!("Bright foreground colors:");
    for code in 90..=97 {
        print!("\x1b[{}m {:>3} \x1b[0m", code, code);
    }
    println!();

    // Background colors
    println!("Background colors:");
    for code in 40..=47 {
        print!("\x1b[{}m {:>3} \x1b[0m", code, code);
    }
    println!();

    // Bright background colors
    println!("Bright background colors:");
    for code in 100..=107 {
        print!("\x1b[{}m {:>3} \x1b[0m", code, code);
    }
    println!();
}

pub fn hide_cursor() {
    print!("\x1b[?25l");
    std::io::stdout().flush().unwrap();
}

pub fn show_cursor() {
    print!("\x1b[?25h");
    std::io::stdout().flush().unwrap();
}
