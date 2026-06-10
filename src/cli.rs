// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/27.

use clap::{CommandFactory, Parser};

#[derive(Parser, Debug)]
#[command(name = "havi", about = "Deterministic HTML-to-video renderer.")]
pub struct Cli {
    /// file://, http(s)://, data: URI, or filesystem path (relative or absolute)
    pub source: Option<String>,
    #[arg(short = 'W', long, default_value_t = 1920)]
    pub width: i32,
    #[arg(short = 'H', long, default_value_t = 1080)]
    pub height: i32,
    #[arg(short, long, default_value_t = 30)]
    pub fps: u32,
    /// Duration in seconds.
    #[arg(short = 't', long, default_value_t = 5)]
    pub duration: u32,
    #[arg(short, long, default_value = "out.mp4")]
    pub out: String,
    /// On load timeout, proceed with partial DOM instead of erroring out.
    #[arg(long)]
    pub tolerant: bool,
    /// HTTP proxy rules (JSON array of {pattern, to, pass, block, status, body, headers}).
    #[arg(long)]
    pub proxy: Option<String>,
    /// Daemon mode: read JSON commands on stdin, emit events on stdout.
    #[arg(long)]
    pub host: bool,
}

pub fn render_help() -> String {
    Cli::command().render_help().to_string()
}
