use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    time::Duration,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub listen_port: u16,
    pub backend_port: u16,
    pub listen_address: IpAddr,
    pub backend_address: IpAddr,
    pub server_address: String,
    /// Exact Minecraft protocol to accept, used when no protocol range is
    /// configured. Defaults to 777 (Minecraft 26.3) for standalone installs.
    pub minecraft_protocol: u16,
    /// Optional inclusive lower bound of accepted Minecraft protocols. Both
    /// bounds must be set together; they override `minecraft_protocol`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_minecraft_protocol: Option<u16>,
    /// Optional inclusive upper bound of accepted Minecraft protocols.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_minecraft_protocol: Option<u16>,
    pub max_connections: usize,
    pub handshake_timeout_secs: u64,
    pub idle_timeout_secs: u64,
    pub connections_per_minute: u32,
    pub diagnostics_interval_secs: u64,
    pub ip_allowlist: Vec<String>,
    pub ip_blocklist: Vec<String>,
    pub pause_menu_button: String,
    pub pause_menu_url: String,
    pub dynamic_notifications: bool,
    pub notification_title: String,
    pub notification_body: String,
    pub notification_source: String,
    pub notification_hide_after_secs: u8,
    pub notification_expire_after_secs: u16,
    pub motd: String,
    pub motd_frames: Vec<String>,
    pub motd_interval_ms: u16,
    pub motd_close_after_ms: u32,
    /// Optional 64x64 PNG data URI. None selects the embedded project icon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motd_icon: Option<String>,
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
            min_minecraft_protocol: None,
            max_minecraft_protocol: None,
            max_connections: 64,
            handshake_timeout_secs: 30,
            idle_timeout_secs: 300,
            connections_per_minute: 30,
            diagnostics_interval_secs: 60,
            ip_allowlist: Vec::new(),
            ip_blocklist: Vec::new(),
            pause_menu_button: String::new(),
            pause_menu_url: String::new(),
            dynamic_notifications: false,
            notification_title: String::new(),
            notification_body: String::new(),
            notification_source: String::new(),
            notification_hide_after_secs: 10,
            notification_expire_after_secs: 3600,
            motd: "EaglerXPumpkin\nMinecraft 26.3".into(),
            motd_frames: Vec::new(),
            motd_interval_ms: 250,
            motd_close_after_ms: 60_000,
            motd_icon: None,
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
        for (name, value, max) in [
            ("handshake_timeout_secs", self.handshake_timeout_secs, 3600),
            ("idle_timeout_secs", self.idle_timeout_secs, 86400),
        ] {
            if value == 0 || value > max {
                return Err(format!("{name} must be between 1 and {max}"));
            }
        }

        if !self.backend_address.is_loopback() {
            return Err("backend_address must be a loopback IP address".into());
        }
        if self.listen_port == self.backend_port
            && (self.listen_address.is_unspecified() || self.listen_address == self.backend_address)
        {
            return Err("listener and backend must use different addresses/ports".into());
        }
        match (self.min_minecraft_protocol, self.max_minecraft_protocol) {
            (None, None) => {}
            (Some(min), Some(max)) => {
                const OLDEST: u16 = 755; // Minecraft 1.17, oldest supported layout
                if min == 0 || max == 0 {
                    return Err(
                        "min_minecraft_protocol and max_minecraft_protocol must be nonzero".into(),
                    );
                }
                if min < OLDEST || max < OLDEST {
                    return Err(format!(
                        "Minecraft protocols below {OLDEST} (1.17) are unsupported; \
                         set min_minecraft_protocol and max_minecraft_protocol to 755 or newer"
                    ));
                }
                if min > max {
                    return Err(
                        "min_minecraft_protocol must not exceed max_minecraft_protocol".into(),
                    );
                }
            }
            _ => {
                return Err(
                    "min_minecraft_protocol and max_minecraft_protocol must both be set \
                     or both be omitted"
                        .into(),
                );
            }
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
        crate::ip_filter::IpFilter::parse(&self.ip_allowlist, &self.ip_blocklist)?;
        if self.pause_menu_button.is_empty() != self.pause_menu_url.is_empty() {
            return Err(
                "pause_menu_button and pause_menu_url must both be set or both empty".into(),
            );
        }
        if !self.pause_menu_button.is_empty() {
            if self.pause_menu_button.len() > 127 {
                return Err("pause_menu_button must be at most 127 bytes".into());
            }
            if !self.pause_menu_url.is_ascii() || self.pause_menu_url.len() > 65535 {
                return Err("pause_menu_url must be ASCII and at most 65535 bytes".into());
            }
        }
        let notification = !self.notification_title.is_empty()
            || !self.notification_body.is_empty()
            || !self.notification_source.is_empty();
        if notification {
            if crate::ui::component(&self.notification_title).len() > 255
                || crate::ui::component(&self.notification_source).len() > 255
            {
                return Err(
                    "serialized notification title and source must be at most 255 bytes".into(),
                );
            }
            if crate::ui::component(&self.notification_body).len() > 32767 {
                return Err("serialized notification_body must be at most 32767 bytes".into());
            }
        }
        if self.server_address.is_empty() || self.server_address.len() > 255 {
            return Err("server_address must contain 1..255 bytes".into());
        }
        if self.motd_frames.len() > 32 {
            return Err("motd_frames must contain at most 32 frames".into());
        }
        for frame in &self.motd_frames {
            if frame.is_empty() || frame.len() > 2048 {
                return Err("each motd_frames entry must contain 1..2048 bytes".into());
            }
        }
        if self.motd_interval_ms > 60_000 {
            return Err("motd_interval_ms must be at most 60000".into());
        }
        if self.motd_close_after_ms > 3_600_000 {
            return Err("motd_close_after_ms must be at most 3600000".into());
        }
        if let Some(icon) = &self.motd_icon {
            crate::motd_icon::decode_data_uri(icon)?;
        }
        if !self.motd_frames.is_empty() {
            if self.motd_interval_ms == 0 {
                return Err(
                    "motd_interval_ms must be between 1 and 60000 when motd_frames is set".into(),
                );
            }
            if self.motd_close_after_ms == 0 {
                return Err(
                    "motd_close_after_ms must be between 1 and 3600000 when motd_frames is set"
                        .into(),
                );
            }
            if (self.motd_close_after_ms as u64) < self.motd_interval_ms as u64 {
                return Err("motd_close_after_ms must be at least motd_interval_ms".into());
            }
        }
        Ok(())
    }

    pub fn listen_addr(&self) -> SocketAddr {
        SocketAddr::new(self.listen_address, self.listen_port)
    }
    pub fn backend_addr(&self) -> SocketAddr {
        SocketAddr::new(self.backend_address, self.backend_port)
    }
    /// Inclusive range of accepted Minecraft protocols. With no range
    /// configured this is exactly `minecraft_protocol`.
    pub fn minecraft_protocol_range(&self) -> (u16, u16) {
        match (self.min_minecraft_protocol, self.max_minecraft_protocol) {
            (Some(min), Some(max)) => (min, max),
            _ => (self.minecraft_protocol, self.minecraft_protocol),
        }
    }
    pub fn pause_menu(&self) -> Option<(&str, &str)> {
        (!self.pause_menu_button.is_empty())
            .then_some((&self.pause_menu_button, &self.pause_menu_url))
    }
    pub fn notification(&self) -> Option<(&str, &str, &str)> {
        let enabled = !self.notification_title.is_empty()
            || !self.notification_body.is_empty()
            || !self.notification_source.is_empty();
        enabled.then_some((
            &self.notification_title,
            &self.notification_body,
            &self.notification_source,
        ))
    }
    /// Animated MOTD settings: frame interval and query max age. Only active
    /// when at least one animation frame is configured.
    pub fn motd_animation(&self) -> Option<(Duration, Duration)> {
        (!self.motd_frames.is_empty()).then_some((
            Duration::from_millis(u64::from(self.motd_interval_ms)),
            Duration::from_millis(u64::from(self.motd_close_after_ms)),
        ))
    }
    /// Server icon as 64x64 RGBA pixels, never a URL in the JSON response.
    pub fn motd_icon(&self) -> Result<Vec<u8>, String> {
        match &self.motd_icon {
            Some(icon) => crate::motd_icon::decode_data_uri(icon),
            None => Ok(crate::builtin_motd_icon()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_timeout_config_defaults_and_bounds() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.handshake_timeout_secs, 30);
        assert_eq!(cfg.idle_timeout_secs, 300);
        for (handshake, idle, valid) in [
            (1, 1, true),
            (3600, 86400, true),
            (0, 300, false),
            (30, 0, false),
            (3601, 300, false),
            (30, 86401, false),
        ] {
            let cfg = Config {
                handshake_timeout_secs: handshake,
                idle_timeout_secs: idle,
                ..Config::default()
            };
            assert_eq!(cfg.validate().is_ok(), valid);
        }
    }

    #[test]
    fn partial_config_keeps_new_defaults() {
        let config: Config = toml::from_str("listen_port = 9000\nmax_connections = 8").unwrap();
        assert_eq!(config.listen_port, 9000);
        assert_eq!(config.max_connections, 8);
        assert_eq!(config.connections_per_minute, 30);
        assert_eq!(config.diagnostics_interval_secs, 60);
        assert!(config.ip_allowlist.is_empty());
        assert!(config.ip_blocklist.is_empty());
        config.validate().unwrap();
    }

    #[test]
    fn minecraft_protocol_range_is_validated_and_defaults_to_exact() {
        // Default: no range, exact protocol only.
        let config = Config::default();
        assert_eq!(config.minecraft_protocol_range(), (777, 777));
        config.validate().unwrap();

        let config: Config =
            toml::from_str("min_minecraft_protocol = 755\nmax_minecraft_protocol = 777").unwrap();
        assert_eq!(config.minecraft_protocol_range(), (755, 777));
        config.validate().unwrap();

        // Half a range is rejected, as is an inverted or too-old range.
        for bad in [
            "min_minecraft_protocol = 755",
            "max_minecraft_protocol = 777",
            "min_minecraft_protocol = 778\nmax_minecraft_protocol = 777",
            "min_minecraft_protocol = 0\nmax_minecraft_protocol = 777",
            "min_minecraft_protocol = 754\nmax_minecraft_protocol = 777",
            "min_minecraft_protocol = 755\nmax_minecraft_protocol = 0",
            "min_minecraft_protocol = 755\nmax_minecraft_protocol = 754",
        ] {
            let config: Config = toml::from_str(bad).unwrap();
            assert!(config.validate().is_err(), "{bad}");
        }
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

    #[test]
    fn rejects_invalid_ip_filter_entries() {
        for config in [
            Config {
                ip_allowlist: vec!["10.0.0.0/33".into()],
                ..Config::default()
            },
            Config {
                ip_blocklist: vec!["not-an-ip".into()],
                ..Config::default()
            },
        ] {
            assert!(config.validate().is_err());
        }
        Config {
            ip_allowlist: vec!["192.168.0.0/16".into(), "::1".into()],
            ip_blocklist: vec!["192.168.1.7".into()],
            ..Config::default()
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn motd_animation_config_ranges_are_validated() {
        let ok = Config {
            motd_frames: vec!["a\nb".into()],
            motd_interval_ms: 50,
            motd_close_after_ms: 60_000,
            ..Config::default()
        };
        ok.validate().unwrap();
        for bad in [
            Config {
                motd_frames: vec!["a".into()],
                motd_interval_ms: 0,
                ..Config::default()
            },
            Config {
                motd_frames: vec!["a".into()],
                motd_interval_ms: 60_001,
                ..Config::default()
            },
            Config {
                motd_frames: vec!["a".into()],
                motd_close_after_ms: 0,
                ..Config::default()
            },
            Config {
                motd_frames: vec!["a".into()],
                motd_close_after_ms: 3_600_001,
                ..Config::default()
            },
            Config {
                motd_frames: vec!["a".into()],
                motd_interval_ms: 100,
                motd_close_after_ms: 50,
                ..Config::default()
            },
            Config {
                motd_frames: vec!["a".into(); 33],
                ..Config::default()
            },
            Config {
                motd_frames: vec![],
                motd_interval_ms: 60_001,
                ..Config::default()
            },
            Config {
                motd_frames: vec![String::from_utf8_lossy(&[b'a'; 2049]).into_owned()],
                ..Config::default()
            },
            Config {
                motd_frames: vec![String::new()],
                ..Config::default()
            },
        ] {
            assert!(bad.validate().is_err());
        }
        // With no frames the interval and close age are unused and may be zero.
        Config {
            motd_interval_ms: 0,
            motd_close_after_ms: 0,
            ..Config::default()
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn motd_icon_defaults_to_the_builtin_server_icon_and_can_be_overridden() {
        let config = Config::default();
        assert!(config.motd_icon.is_none());
        assert_eq!(config.motd_icon().unwrap(), crate::builtin_motd_icon());

        let override_config = Config {
            motd_icon: Some(crate::motd_icon::tests::test_icon_uri()),
            ..Config::default()
        };
        override_config.validate().unwrap();
        assert_eq!(
            override_config.motd_icon().unwrap(),
            [17, 34, 51, 68].repeat(4096)
        );

        for bad in [
            Config {
                motd_icon: Some("https://example.com/icon.png".into()),
                ..Config::default()
            },
            Config {
                motd_icon: Some(String::new()),
                ..Config::default()
            },
            Config {
                motd_icon: Some("é".into()),
                ..Config::default()
            },
            Config {
                motd_icon: Some("a".repeat(2_097_153)),
                ..Config::default()
            },
        ] {
            assert!(bad.validate().is_err(), "invalid motd_icon accepted");
        }
    }

    #[test]
    fn pause_menu_and_notification_config_is_optional_and_validated() {
        let mut config = Config {
            pause_menu_button: "Discord".into(),
            pause_menu_url: "https://example.com/invite".into(),
            notification_title: "Welcome".into(),
            notification_body: "Have fun".into(),
            notification_source: "EaglerXPumpkin".into(),
            ..Config::default()
        };
        assert_eq!(
            config.pause_menu(),
            Some(("Discord", "https://example.com/invite"))
        );
        assert!(config.notification().is_some());
        config.validate().unwrap();

        config.pause_menu_url.clear();
        assert!(config.validate().is_err());
        config.pause_menu_url = "https://example.com".into();
        config.pause_menu_button = "é".into();
        assert!(config.pause_menu_url.is_ascii());
        assert!(config.validate().is_ok()); // non-ASCII button text is UTF-8 encoded

        let empty = Config::default();
        assert_eq!(empty.pause_menu(), None);
        assert_eq!(empty.notification(), None);
        empty.validate().unwrap();
    }
}
