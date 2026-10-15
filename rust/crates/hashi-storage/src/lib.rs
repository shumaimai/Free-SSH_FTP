//! GUIに依存しない保存先と移行前の停止確認。

pub mod config;
pub mod crypto;
pub mod legacy_permissions;
pub mod migration;
pub mod paths;
pub mod secrets;
#[cfg(windows)]
pub mod winvault;
