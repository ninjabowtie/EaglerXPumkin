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
