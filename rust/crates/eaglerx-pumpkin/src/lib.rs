//! Eaglercraft WebSocket gateway for Pumpkin, polled without guest threads.
pub mod config;
pub mod ip_filter;
pub mod minecraft;
pub mod notifications;
pub mod runtime;
mod session;
mod skins;
mod ui;

mod motd_icon;
#[cfg(target_os = "wasi")]
mod notification_command;

/// The repository logo resized to 64x64 for Eagler server-list queries.
pub const EAGLER_ICON: &[u8] = include_bytes!("../assets/motd-icon.png");

/// Built-in server icon as 4096 RGBA pixels in Eagler wire order.
pub fn builtin_motd_icon() -> Vec<u8> {
    static PIXELS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PIXELS
        .get_or_init(|| motd_icon::decode_png(EAGLER_ICON).expect("valid built-in MOTD icon"))
        .clone()
}

#[cfg(target_os = "wasi")]
mod plugin {
    use crate::{config::Config, runtime::Runtime};
    use pumpkin_plugin_api::{
        Context, Plugin, PluginMetadata, permissions,
        scheduler::{self, SchedulerExt},
    };
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };
    struct Running {
        task: u32,
        runtime: Arc<Mutex<Option<Runtime>>>,
    }
    struct EaglerXPlugin {
        running: Mutex<Option<Running>>,
    }
    impl Plugin for EaglerXPlugin {
        fn new() -> Self {
            Self {
                running: Mutex::new(None),
            }
        }
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata {
                name: "eaglerx-pumpkin".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                authors: vec![
                    "EaglerXPumpkin contributors".into(),
                    "lax1dude (original EaglerXServer)".into(),
                ],
                description: "Eaglercraft gateway for Minecraft 26.3 on Pumpkin".into(),
                dependencies: vec![],
                permissions: vec![
                    permissions::NETWORK_TCP_BIND.into(),
                    permissions::NETWORK_TCP_CONNECT.into(),
                    permissions::FS_READ_DATA.into(),
                    permissions::FS_WRITE_DATA.into(),
                ],
            }
        }
        fn on_load(&self, context: Context) -> Result<(), String> {
            if self.running.lock().map_err(|e| e.to_string())?.is_some() {
                return Err("plugin already loaded".into());
            }
            let path = PathBuf::from(context.get_data_folder()).join("eagler.toml");
            let config = Config::load(&path)?;
            let dynamic_notifications = config.dynamic_notifications;
            let runtime = Arc::new(Mutex::new(Some(
                Runtime::bind(config).map_err(|e| format!("listener failed: {e}"))?,
            )));
            if dynamic_notifications {
                crate::notification_command::register(&context, Arc::clone(&runtime))?;
            }
            let task_runtime = Arc::clone(&runtime);
            let task = context.schedule_repeating_task(1, 1, move |_server| {
                if let Ok(mut runtime) = task_runtime.lock()
                    && let Some(runtime) = runtime.as_mut()
                {
                    runtime.poll();
                }
            });
            *self.running.lock().map_err(|e| e.to_string())? = Some(Running { task, runtime });
            tracing::info!("EaglerXPumpkin loaded; scheduler listener active");
            Ok(())
        }
        fn on_unload(&self, _context: Context) -> Result<(), String> {
            let running = self.running.lock().map_err(|e| e.to_string())?.take();
            if let Some(running) = running {
                running.runtime.lock().map_err(|e| e.to_string())?.take();
                scheduler::cancel_task(running.task);
            }
            Ok(())
        }
    }
    pumpkin_plugin_api::register_plugin!(EaglerXPlugin);
}
