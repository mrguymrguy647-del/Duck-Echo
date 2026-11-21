//! Duck Echo Studio: make sound effects and music without a terminal.
//!
//! This crate is the window and what only an app needs: the speakers, the
//! settings, the new-version check and the Linux app-menu entry. What a
//! sound does lives in the Duck Echo libraries (`duck-echo-tools`,
//! `duck-echo-live` and the rest), the very code an AI assistant runs
//! through the MCP server.

pub mod app;
pub mod app_menu;
pub mod connect;
pub mod controls;
pub mod ear_panel;
pub mod editor;
pub mod home;
pub mod library;
pub mod opening;
pub mod send;
pub mod settings;
pub mod song_edit;
pub mod song_screen;
pub mod song_view;
pub mod sound_screen;
pub mod speaker;
pub mod theme;
pub mod together;
pub mod update;
pub mod update_banner;

pub use app::{AppMenu, StudioApp};
