mod auth;
mod config;
mod graph;
mod handler;
mod protocol;

use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<protocol::Request>(&line) {
            Ok(request) => handler::handle(request),
            Err(e) => serde_json::json!({ "error": format!("Bad request: {e}") }),
        };

        let _ = writeln!(out, "{response}");
        let _ = out.flush();
    }
}
