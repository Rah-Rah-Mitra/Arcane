//! Formatted terminal output helpers.
//!
//! Centralised formatting functions for consistent CLI output.

/// Print a section header.
#[allow(dead_code)]
pub fn print_header(text: &str) {
    println!("\n{text}");
    println!("{}", "─".repeat(text.len()));
}
