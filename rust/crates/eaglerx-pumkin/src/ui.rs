//! Optional Eagler v5 pause-menu and notification game messages.
//!
//! Packet layouts follow EaglerXServer's `SPacketCustomizePauseMenuV4EAG` and
//! `SPacketNotifBadgeShowV4EAG`. Only the subset needed for a static join
//! message is produced: an external pause-menu link and one text notification
//! without custom icons or image mappings.
use crate::minecraft as mc;

pub const CAP_NOTIFICATION: u32 = 1 << 3;
pub const CAP_PAUSE_MENU: u32 = 1 << 4;

const PACKET_PAUSE_MENU: u8 = 0x18;
const PACKET_NOTIFICATION: u8 = 0x1F;
const SERVER_INFO_MODE_EXTERNAL_URL: u8 = 1;
const BADGE_PRIORITY_NORMAL: u8 = 1;
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// Stable ID so reconnects replace the same notification instead of stacking.
const JOIN_BADGE: [u8; 16] = [
    0xE7, 0x2A, 0x1B, 0x59, 0x53, 0x6B, 0x4F, 0x0D, 0x9A, 0x27, 0x6B, 0x4E, 0x32, 0x50, 0x55, 0x4D,
];

fn mc_string(out: &mut Vec<u8>, value: &str) {
    mc::varint(out, value.len() as i32);
    out.extend_from_slice(value.as_bytes());
}

fn ascii16(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.chars().count() as u16).to_be_bytes());
    out.extend(value.chars().map(|ch| {
        let code = ch as u32;
        if code <= 0xFF { code as u8 } else { b'?' }
    }));
}

/// Minecraft JSON text component for plain notification text.
pub fn component(text: &str) -> String {
    serde_json::json!({ "text": text }).to_string()
}

/// `SPacketCustomizePauseMenuV4EAG` with an external "server info" link.
pub fn pause_menu(button: &str, url: &str) -> Vec<u8> {
    let mut out = vec![PACKET_PAUSE_MENU, SERVER_INFO_MODE_EXTERNAL_URL];
    mc_string(&mut out, button);
    ascii16(&mut out, url);
    mc::varint(&mut out, 0); // no image mappings
    out
}

/// `SPacketNotifBadgeShowV4EAG` with default white text and no custom icons.
pub fn notification(
    title: &str,
    body: &str,
    source: &str,
    hide_after_secs: u8,
    expire_after_secs: u16,
    timestamp_secs: u64,
) -> Vec<u8> {
    let mut out = vec![PACKET_NOTIFICATION];
    out.extend_from_slice(&JOIN_BADGE);
    mc_string(&mut out, &component(body));
    mc_string(&mut out, &component(title));
    mc_string(&mut out, &component(source));
    out.extend_from_slice(&((timestamp_secs >> 32) as u16).to_be_bytes());
    out.extend_from_slice(&(timestamp_secs as u32).to_be_bytes());
    out.push(BADGE_PRIORITY_NORMAL << 1);
    out.push(hide_after_secs);
    out.extend_from_slice(&expire_after_secs.to_be_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&WHITE);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_menu_uses_external_link_mode() {
        let packet = pause_menu("Join us", "https://example.com");
        let mut r = mc::Reader::new(&packet);
        assert_eq!(r.byte().unwrap(), PACKET_PAUSE_MENU);
        assert_eq!(r.byte().unwrap(), SERVER_INFO_MODE_EXTERNAL_URL);
        assert_eq!(r.text(127).unwrap(), "Join us");
        assert_eq!(r.short().unwrap(), 19);
        assert_eq!(r.take(19).unwrap(), b"https://example.com");
        assert_eq!(r.int().unwrap(), 0); // image mappings
        r.end().unwrap();
    }

    #[test]
    fn notification_has_text_components_and_default_colors() {
        let packet = notification("Title", "Body", "Server", 10, 3600, 0x1_0000_0002);
        let mut r = mc::Reader::new(&packet);
        assert_eq!(r.byte().unwrap(), PACKET_NOTIFICATION);
        assert_eq!(r.take(16).unwrap(), JOIN_BADGE);
        assert_eq!(r.text(32767).unwrap(), "{\"text\":\"Body\"}");
        assert_eq!(r.text(255).unwrap(), "{\"text\":\"Title\"}");
        assert_eq!(r.text(255).unwrap(), "{\"text\":\"Server\"}");
        assert_eq!(r.short().unwrap(), 1);
        assert_eq!(
            u32::from_be_bytes(r.take(4).unwrap().try_into().unwrap()),
            2
        );
        assert_eq!(r.byte().unwrap(), 2); // NORMAL priority, not silent
        assert_eq!(r.byte().unwrap(), 10);
        assert_eq!(r.short().unwrap(), 3600);
        assert_eq!(r.take(12).unwrap(), [0xFF; 12]);
        r.end().unwrap();
    }
}
