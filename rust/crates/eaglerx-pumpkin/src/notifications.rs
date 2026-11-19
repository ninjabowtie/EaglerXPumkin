//! Validated dynamic notification requests, shared by the Pumpkin command and
//! native embedders. Text is always literal, never interpreted as JSON markup.
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

/// Eagler's four badge priorities, encoded in the notification flags.
#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Priority {
    Low = 0,
    #[default]
    Normal = 1,
    Higher = 2,
    Highest = 3,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Notification {
    /// Reusing an ID replaces the same badge. Distinct IDs can coexist.
    pub id: String,
    pub title: String,
    pub body: String,
    pub source: String,
    pub priority: Priority,
    pub silent: bool,
    pub hide_after_secs: u8,
    pub expire_after_secs: u16,
}

impl Default for Notification {
    fn default() -> Self {
        Self {
            id: "server".into(),
            title: String::new(),
            body: String::new(),
            source: "EaglerXPumpkin".into(),
            priority: Priority::Normal,
            silent: false,
            hide_after_secs: 10,
            expire_after_secs: 3600,
        }
    }
}

impl Notification {
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > 65536 {
            return Err("notification JSON must be at most 65536 bytes".into());
        }
        let request: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_id(&self.id)?;
        if self.title.is_empty() && self.body.is_empty() {
            return Err("notification needs a title or body".into());
        }
        if crate::ui::component(&self.title).len() > 255
            || crate::ui::component(&self.source).len() > 255
            || crate::ui::component(&self.body).len() > 32767
        {
            return Err("serialized title/source must fit 255 bytes and body 32767 bytes".into());
        }
        if self.hide_after_secs == 0 || self.expire_after_secs < u16::from(self.hide_after_secs) {
            return Err("hide_after_secs must be positive and not exceed expire_after_secs".into());
        }
        Ok(())
    }

    pub(crate) fn badge_id(&self) -> [u8; 16] {
        badge_id(&self.id)
    }

    pub(crate) fn packet(&self, timestamp: u64) -> Vec<u8> {
        crate::skins::injected(crate::ui::notification_with_options(
            &self.badge_id(),
            self,
            timestamp,
        ))
    }
}

pub(crate) fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 {
        Err("notification id must contain 1..64 bytes".into())
    } else {
        Ok(())
    }
}

fn badge_id(name: &str) -> [u8; 16] {
    let mut id: [u8; 16] = Md5::digest(format!("EaglerXPumpkin:notification:{name}")).into();
    id[6] = (id[6] & 15) | 0x30;
    id[8] = (id[8] & 63) | 0x80;
    id
}

/// Dismiss a command-created badge by the same human-readable ID used to show it.
/// Does not affect the separate static welcome badge.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DismissNotification {
    pub id: String,
}

impl DismissNotification {
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > 1024 {
            return Err("dismiss JSON must be at most 1024 bytes".into());
        }
        let request: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        validate_id(&request.id)?;
        Ok(request)
    }

    pub(crate) fn packet(&self) -> Result<Vec<u8>, String> {
        validate_id(&self.id)?;
        let mut packet = vec![0x20];
        packet.extend_from_slice(&badge_id(&self.id));
        Ok(crate::skins::injected(packet))
    }
}

pub(crate) fn validate_target(target: &str) -> Result<(), String> {
    if target == "*"
        || (!target.is_empty()
            && target.len() <= 16
            && target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    {
        Ok(())
    } else {
        Err("target must be an exact Minecraft username or * for broadcast".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn priorities_and_silent_flags_follow_reference_layout() {
        for (name, bits) in [("low", 0), ("normal", 2), ("higher", 4), ("highest", 6)] {
            for silent in [false, true] {
                let request = Notification::from_json(&format!(
                    r#"{{"title":"x","priority":"{name}","silent":{silent}}}"#
                ))
                .unwrap();
                let packet = request.packet(0x1234_5678_9abc);
                let mut r = crate::minecraft::Reader::new(&packet[18..]);
                r.text(32767).unwrap();
                r.text(255).unwrap();
                r.text(255).unwrap();
                assert_eq!(r.short().unwrap(), 0x1234);
                assert_eq!(r.take(4).unwrap(), &[0x56, 0x78, 0x9a, 0xbc]);
                assert_eq!(r.byte().unwrap(), bits | u8::from(silent));
                assert_eq!(r.byte().unwrap(), 10);
                assert_eq!(r.short().unwrap(), 3600);
                assert_eq!(r.take(12).unwrap(), &[255; 12]);
                r.end().unwrap();
            }
        }
        assert!(Notification::from_json(r#"{"title":"x","priority":"urgent"}"#).is_err());
        assert!(Notification::from_json(r#"{"title":"x","priority":3}"#).is_err());
    }

    #[test]
    fn hide_packet_matches_show_id_and_rejects_invalid_requests() {
        let show = Notification {
            id: "maintenance".into(),
            title: "x".into(),
            ..Default::default()
        };
        let hide = DismissNotification::from_json(r#"{"id":"maintenance"}"#)
            .unwrap()
            .packet()
            .unwrap();
        assert_eq!(&hide[..2], &[0xee, 0x20]);
        assert_eq!(&hide[2..], &show.packet(0)[2..18]);
        assert_eq!(hide.len(), 18);
        for json in [r#"{}"#, r#"{"id":""}"#, r#"{"id":"a","unknown":true}"#] {
            assert!(DismissNotification::from_json(json).is_err());
        }
        assert!(DismissNotification { id: "a".repeat(65) }.packet().is_err());
        assert!(DismissNotification::from_json(&" ".repeat(1025)).is_err());
    }

    #[test]
    fn validates_json_and_escaped_component_lengths() {
        assert!(Notification::from_json(r#"{"title":"Hello","body":"World"}"#).is_ok());
        for json in [
            r#"{}"#,
            r#"{"title":"x","unknown":true}"#,
            r#"{"title":"x","hide_after_secs":0}"#,
            r#"{"title":"x","expire_after_secs":1}"#,
            r#"{"title":"x","id":""}"#,
        ] {
            assert!(Notification::from_json(json).is_err(), "{json}");
        }
        let n = Notification {
            title: "\"".repeat(200),
            ..Notification::default()
        };
        assert!(n.validate().is_err());
        assert!(Notification::from_json(&" ".repeat(65537)).is_err());
        assert!(validate_target("*").is_ok());
        assert!(validate_target("Player_1").is_ok());
        for target in ["", "@a", "a b", "é", "ThisNameIsTooLong123"] {
            assert!(validate_target(target).is_err());
        }
    }
    #[test]
    fn stable_ids_replace_badges_and_text_is_literal() {
        let n = Notification {
            title: "<alert>".into(),
            body: "你好".into(),
            ..Notification::default()
        };
        let mut updated = n.clone();
        updated.body = "Updated".into();
        assert_eq!(n.badge_id(), updated.badge_id());
        updated.id = "another".into();
        assert_ne!(n.badge_id(), updated.badge_id());
        let packet = n.packet(123);
        assert_eq!(&packet[..2], &[0xee, 0x1f]);
        assert_eq!(&packet[2..18], &n.badge_id());
        let mut r = crate::minecraft::Reader::new(&packet[18..]);
        assert_eq!(r.text(32767).unwrap(), r#"{"text":"你好"}"#);
        assert_eq!(r.text(255).unwrap(), r#"{"text":"<alert>"}"#);
    }
}
