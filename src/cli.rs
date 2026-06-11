// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/27.

use clap::{CommandFactory, Parser};

#[derive(Parser, Debug)]
#[command(name = "havi", about = "Deterministic HTML-to-video renderer.")]
pub struct Cli {
    /// file://, http(s)://, data: URI, or filesystem path (relative or absolute)
    pub source: String,
    #[arg(short = 'W', long, default_value_t = 1920)]
    pub width: i32,
    #[arg(short = 'H', long, default_value_t = 1080)]
    pub height: i32,
    #[arg(short, long, default_value_t = 30)]
    pub fps: u32,
    /// Duration in seconds.
    #[arg(short = 't', long, default_value_t = 5)]
    pub duration: u32,
    /// Comma-separated output paths; extension picks the codec (.mp4 = HEVC+alpha,
    /// .webm = VP9+alpha). Default writes both.
    #[arg(short, long, default_value = "out.mp4,out.webm")]
    pub out: String,
    /// On load timeout, proceed with partial DOM instead of erroring out.
    #[arg(long)]
    pub tolerant: bool,
    /// HTTP proxy rules (JSON array of {pattern, to, pass, block, status, body, headers}).
    #[arg(long)]
    pub proxy: Option<String>,
}

pub fn render_help() -> String {
    Cli::command().render_help().to_string()
}
