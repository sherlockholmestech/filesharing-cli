use owo_colors::{OwoColorize, Style};
use std::io::{self, IsTerminal};

fn supports_color() -> bool {
    io::stderr().is_terminal() && io::stdout().is_terminal()
}

fn if_color<T>(styled: T, plain: T) -> T {
    if supports_color() { styled } else { plain }
}

pub fn ok_prefix() -> String {
    if_color("  ✓ ".green().to_string(), "  ✓ ".to_string())
}

pub fn err_prefix() -> String {
    if_color("  ✗ ".red().to_string(), "  ✗ ".to_string())
}

pub fn error(msg: impl AsRef<str>) -> String {
    format!("{}{}", err_prefix(), msg.as_ref().red())
}

pub fn url(msg: impl AsRef<str>) -> String {
    if_color(
        msg.as_ref().blue().underline().to_string(),
        msg.as_ref().to_string(),
    )
}

pub fn dim(msg: impl AsRef<str>) -> String {
    if_color(
        msg.as_ref().style(Style::new().bright_black()).to_string(),
        msg.as_ref().to_string(),
    )
}
