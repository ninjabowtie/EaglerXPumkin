use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub listen_port: u16,
    pub backend_port: u16,
    pub listen_address: IpAddr,
    pub backend_address: IpAddr,
    pub server_address: String,
    pub minecraft_protocol: u16,
    pub max_connections: usize,
    pub connections_per_minute: u32,
    pub diagnostics_interval_secs: u64,
    pub motd: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_port: 8081,
            backend_port: 25565,
            listen_address: Ipv4Addr::UNSPECIFIED.into(),
            backend_address: Ipv4Addr::LOCALHOST.into(),
            server_address: "localhost".into(),
            minecraft_protocol: 777, // Minecraft 26.3
            max_connections: 64,
            connections_per_minute: 30,
            diagnostics_interval_secs: 60,
            motd: "EaglerXPumkin\nMinecraft 26.3".into(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let config = match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str::<Self>(&text).map_err(|e| format!("{}: {e}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::write(
                    path,
                    toml::to_string_pretty(&config).map_err(|e| e.to_string())?,
                )
                .map_err(|e| format!("{}: {e}", path.display()))?;
                config
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.backend_address.is_loopback() {
            return Err("backend_address must be a loopback IP address".into());
        }
        if self.listen_port == self.backend_port
            && (self.listen_address.is_unspecified() || self.listen_address == self.backend_address)
        {
            return Err("listener and backend must use different addresses/ports".into());
        }
        if self.max_connections == 0 || self.max_connections > 256 {
            return Err("max_connections must be between 1 and 256".into());
        }
        if self.connections_per_minute > 3600 {
            return Err(
                "connections_per_minute must be at most 3600 (0 disables the limit)".into(),
            );
        }
        if self.diagnostics_interval_secs > 3600 {
            return Err(
                "diagnostics_interval_secs must be at most 3600 (0 disables reports)".into(),
            );
        }
        if self.server_address.is_empty() || self.server_address.len() > 255 {
            return Err("server_address must contain 1..255 bytes".into());
        }
        Ok(())
    }

    pub fn listen_addr(&self) -> SocketAddr {
        SocketAddr::new(self.listen_address, self.listen_port)
    }
    pub fn backend_addr(&self) -> SocketAddr {
        SocketAddr::new(self.backend_address, self.backend_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_keeps_new_defaults() {
        let config: Config = toml::from_str("listen_port = 9000\nmax_connections = 8").unwrap();
        assert_eq!(config.listen_port, 9000);
        assert_eq!(config.max_connections, 8);
        assert_eq!(config.connections_per_minute, 30);
        assert_eq!(config.diagnostics_interval_secs, 60);
        config.validate().unwrap();
    }

    #[test]
    fn zero_rate_limit_and_report_interval_disable_them() {
        let config = Config {
            connections_per_minute: 0,
            diagnostics_interval_secs: 0,
            ..Config::default()
        };
        config.validate().unwrap();
    }

    #[test]
    fn rejects_out_of_range_diagnostics_settings() {
        for config in [
            Config {
                connections_per_minute: 3601,
                ..Config::default()
            },
            Config {
                diagnostics_interval_secs: 3601,
                ..Config::default()
            },
        ] {
            assert!(config.validate().is_err());
        }
    }
}
