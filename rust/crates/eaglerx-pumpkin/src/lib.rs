//! Eaglercraft WebSocket gateway for Pumpkin, polled without guest threads.
pub mod config;
pub mod ip_filter;
pub mod minecraft;
pub mod runtime;
mod session;
mod skins;
mod ui;

/// Server icon for EaglerXPumpkin, embedded from the repository image
/// (`EaglerXPumkin.png`) as base64 so MOTD/server-list answers can override the
/// backend's default icon without any runtime fetch.
pub const EAGLER_ICON: &[u8] = include_bytes!("../../../../EaglerXPumpkin.png");

/// Built-in MOTD/server icon for EaglerXPumpkin: the repository's
/// `EaglerXPumpkin.png` as a base64 data URI, used when no admin override is
/// configured.
pub fn builtin_motd_icon() -> String {
    format!(
        "data:image/png;base64,{}",
        eagler_ws::base64_encode(EAGLER_ICON)
    )
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
            let runtime = Arc::new(Mutex::new(Some(
                Runtime::bind(config).map_err(|e| format!("listener failed: {e}"))?,
            )));
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
