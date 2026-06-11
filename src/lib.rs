// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

#[cfg(feature = "napi-binding")]
#[macro_use]
extern crate napi_derive;

pub mod api;
pub mod cef;
pub mod cli;
pub mod common;
pub mod proxy;
pub mod renderer;
pub mod video;

#[cfg(feature = "napi-binding")]
pub mod napi;

pub use common::{ipc, sandbox};
pub use sandbox::{cleanup_session, install_cleanup_hooks, install_parent_death_watcher, register_ffmpeg, sandbox_dir, scratch_dir, unregister_ffmpeg};
