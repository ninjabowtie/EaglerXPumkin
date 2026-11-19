// Eagler handshake ported from EaglerXServer HandshakerV2–V5 and
// WebSocketEaglerInitialHandler. Copyright (c) 2022–2025 lax1dude.
// Distributed under the license in the repository root.
use crate::{
    config::Config,
    minecraft::{self as mc, Codec, Reader, Result},
    skins::{self, PlayerSkin, SkinStore},
    ui,
};
use eagler_ws::{Message, OP_BIN, OP_TEXT};
use md5::{Digest, Md5};
use std::{
    collections::HashSet,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, PartialEq, Eq)]
enum Stage {
    Version,
    Request,
    BackendLogin,
    Profiles,
    Play,
    Motd,
}

/// Per-connection MOTD animation state, ported from EaglerMOTD's
/// EaglerMOTDConnectionUpdater. The poll loop replaces its tick timer: each
/// poll cycles to the next frame once the interval has elapsed and
/// force-closes the query connection after the max age.
struct MotdAnim {
    frame: usize,
    interval: Duration,
    close_after: Duration,
    started: Instant,
    next_frame: Instant,
}

pub struct Session {
    stage: Stage,
    version: u16,
    protocol: u16,
    username: String,
    handshake_username: Option<String>,
    uuid: [u8; 16],
    codec: Codec,
    profiles: HashSet<String>,
    profile_bytes: usize,
    player_skin: PlayerSkin,
    capabilities: u32,
    motd: Option<MotdAnim>,
    stalled: Vec<Vec<u8>>,
    stalled_bytes: usize,
    pub client_output: Vec<Message>,
    pub backend_output: Vec<u8>,
    pub connect_requested: bool,
    pub closing: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            stage: Stage::Version,
            version: 0,
            protocol: 0,
            username: String::new(),
            handshake_username: None,
            uuid: [0; 16],
            codec: Codec::default(),
            profiles: HashSet::new(),
            profile_bytes: 0,
            player_skin: PlayerSkin::default(),
            capabilities: 0,
            motd: None,
            stalled: Vec::new(),
            stalled_bytes: 0,
            client_output: Vec::new(),
            backend_output: Vec::new(),
            connect_requested: false,
            closing: false,
        }
    }
}

fn ascii(out: &mut Vec<u8>, text: &str) {
    out.push(text.len() as u8);
    out.extend_from_slice(text.as_bytes());
}

/// Serialize one MOTD query response. Animated frames disable client-side
/// caching (so the browser re-queries the sequence) and carry the reference's
/// epoch-millisecond timestamp; static answers keep the verified shape and use
/// the configured server icon, defaulting to the built-in EaglerXPumpkin icon.
fn motd_frame(cfg: &Config, online: usize, frame: &[&str], animated: bool, icon: bool) -> Vec<u8> {
    let time = if animated {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0)
    } else {
        0
    };
    serde_json::json!({
        "name": "EaglerXPumpkin", "brand": "EaglerXPumpkin", "vers": env!("CARGO_PKG_VERSION"),
        "cracked": true, "secure": false, "type": "motd", "time": time,
        "data": { "motd": frame, "cache": !animated, "icon": icon,
            "online": online, "max": cfg.max_connections, "players": [] }
    })
    .to_string()
    .into_bytes()
}

/// Optional capabilities this gateway can provide with its current config.
fn optional_capabilities(cfg: &Config) -> u32 {
    let mut caps = 0;
    if cfg.pause_menu().is_some() {
        caps |= ui::CAP_PAUSE_MENU;
    }
    if cfg.notification().is_some() {
        caps |= ui::CAP_NOTIFICATION;
    }
    caps
}

/// Display names for the accepted Minecraft protocol range. Protocols outside
/// the table label by number alone.
fn minecraft_protocol_name(protocol: u16) -> Option<&'static str> {
    match protocol {
        755 => Some("1.17"),
        756 => Some("1.17.1"),
        757 => Some("1.18"),
        758 => Some("1.18.2"),
        759 => Some("1.19"),
        760 => Some("1.19.2"),
        761 => Some("1.19.3"),
        762 => Some("1.19.4"),
        763 => Some("1.20/1.20.1"),
        764 => Some("1.20.2"),
        765 => Some("1.20.3/1.20.4"),
        766 => Some("1.20.5/1.20.6"),
        767 => Some("1.21/1.21.1"),
        768 => Some("1.21.2/1.21.3"),
        769 => Some("1.21.4"),
        770 => Some("1.21.5"),
        771 => Some("1.21.6"),
        772 => Some("1.21.7/1.21.8"),
        773 => Some("1.21.9/1.21.10"),
        774 => Some("1.21.11"),
        775 => Some("26.1"),
        776 => Some("26.2"),
        777 => Some("26.3"),
        _ => None,
    }
}

/// The actionable message carried by the mismatch packet, e.g.
/// "Use a Minecraft 1.17-26.3 Eaglercraft client (protocols 755-777)".
fn mismatch_message(min: u16, max: u16) -> String {
    let versions = match (
        minecraft_protocol_name(min),
        minecraft_protocol_name(max),
        min == max,
    ) {
        (Some(name), _, true) => name.to_string(),
        (Some(min_name), Some(max_name), false) => format!("{min_name}-{max_name}"),
        _ => format!("protocol {min}-{max}"),
    };
    let protocols = if min == max {
        format!(" (protocol {min})")
    } else {
        format!(" (protocols {min}-{max})")
    };
    format!("Use a Minecraft {versions} Eaglercraft client{protocols}")
}

/// Build the handshake-version mismatch packet (id 0x03): the Eagler versions
/// this server speaks, the accepted Minecraft protocols (min and max when they
/// differ), and the actionable message. Ported from
/// WebSocketEaglerInitialHandler's incompatibility reply.
fn mismatch_packet(cfg: &Config) -> Vec<u8> {
    let (min, max) = cfg.minecraft_protocol_range();
    let mut packet = vec![3];
    packet.extend_from_slice(&1u16.to_be_bytes()); // supported Eagler versions
    packet.extend_from_slice(&5u16.to_be_bytes());
    if min == max {
        packet.extend_from_slice(&1u16.to_be_bytes());
        packet.extend_from_slice(&min.to_be_bytes());
    } else {
        packet.extend_from_slice(&2u16.to_be_bytes());
        packet.extend_from_slice(&min.to_be_bytes());
        packet.extend_from_slice(&max.to_be_bytes());
    }
    ascii(&mut packet, &mismatch_message(min, max));
    packet
}

/// Append the version-specific Login Start fields after the username, per the
/// wiki.vg 1.17+ layouts: 1.19 adds a signature-data flag, 1.19.2 adds that
/// flag plus the optional player UUID, 1.19.3-1.20.1 keep the optional UUID
/// behind its own flag, and 1.20.2+ carry an unconditional UUID.
fn login_start(out: &mut Vec<u8>, protocol: u16, uuid: &[u8; 16]) {
    match protocol {
        755..=758 => {}     // username only
        759 => out.push(0), // has_signature_data = false
        760 => {
            out.push(0); // has_signature_data = false
            out.push(1); // has_player_uuid = true
            out.extend_from_slice(uuid);
        }
        761..=763 => {
            out.push(1); // has_player_uuid = true
            out.extend_from_slice(uuid);
        }
        _ => out.extend_from_slice(uuid), // 1.20.2+: required player UUID
    }
}

impl Session {
    fn send(&mut self, payload: Vec<u8>) {
        self.client_output.push(Message {
            opcode: OP_BIN,
            payload,
        });
    }
    pub fn is_playing(&self) -> bool {
        self.stage == Stage::Play
    }
    /// Whether this connection is an animated MOTD query the poll loop must
    /// drive until its max age.
    pub fn is_animating_motd(&self) -> bool {
        self.motd.is_some()
    }
    /// Push one animation frame as a text WebSocket message.
    fn send_motd(&mut self, cfg: &Config, online: usize, frame: usize) {
        let lines: Vec<&str> = cfg.motd_frames[frame].lines().take(2).collect();
        self.client_output.push(Message {
            opcode: OP_TEXT,
            payload: motd_frame(cfg, online, &lines, true, false),
        });
    }
    /// Advance the MOTD animation one poll: cycle to the next frame when the
    /// interval has elapsed and force-close once the max age is reached.
    /// Returns after at most one frame per poll, matching EaglerMOTD's tick
    /// granularity.
    pub fn motd_tick(&mut self, cfg: &Config, online: usize, now: Instant) {
        if self.motd.is_none() {
            return;
        }
        if self.closing {
            self.motd = None;
            return;
        }
        let expired = match &self.motd {
            Some(anim) => {
                if now < anim.next_frame {
                    return;
                }
                now.duration_since(anim.started) >= anim.close_after
            }
            None => return,
        };
        if expired {
            self.closing = true;
            self.motd = None;
            return;
        }
        let anim = self.motd.as_mut().unwrap();
        anim.next_frame = now + anim.interval;
        anim.frame = (anim.frame + 1) % cfg.motd_frames.len();
        let frame = anim.frame;
        self.send_motd(cfg, online, frame);
    }
    pub fn uuid(&self) -> [u8; 16] {
        self.uuid
    }
    pub fn fail(&mut self, reason: &str) {
        if self.closing {
            return;
        }
        tracing::warn!("Eagler session failed: {reason}");
        if self.stage == Stage::Play {
            self.closing = true;
            return;
        }
        let text = if self.version >= 3 {
            serde_json::json!({"text": reason}).to_string()
        } else {
            reason.to_string()
        };
        let text = &text.as_bytes()[..text.len().min(65535)];
        let mut packet = vec![0xff, 8];
        if self.version >= 3 {
            packet.extend_from_slice(&(text.len() as u16).to_be_bytes());
        } else {
            packet.push(text.len().min(255) as u8);
        }
        packet.extend_from_slice(if self.version >= 3 {
            text
        } else {
            &text[..text.len().min(255)]
        });
        self.send(packet);
        self.closing = true;
    }

    pub fn client_message(
        &mut self,
        message: Message,
        cfg: &Config,
        online: usize,
        store: &mut SkinStore,
    ) -> Result<()> {
        if self.closing {
            return Ok(());
        }
        if message.opcode == OP_TEXT && self.stage == Stage::Version {
            let request = std::str::from_utf8(&message.payload).map_err(|_| "invalid query")?;
            if !request.to_ascii_lowercase().starts_with("accept: motd") {
                return Err("unsupported query".into());
            }
            if let Some((interval, close_after)) = cfg.motd_animation() {
                // Animated MOTD, ported from EaglerMOTD: answer with the first
                // frame and keep the query connection open. The poll loop
                // drives the remaining frames and the max-age close.
                let now = Instant::now();
                self.motd = Some(MotdAnim {
                    frame: 0,
                    interval,
                    close_after,
                    started: now,
                    next_frame: now + interval,
                });
                self.send_motd(cfg, online, 0);
                self.stage = Stage::Motd;
                return Ok(());
            }
            let lines: Vec<_> = cfg.motd.lines().take(2).collect();
            let no_icon = request.split_once('.').is_some_and(|(_, subtype)| {
                let subtype = subtype.to_ascii_lowercase();
                subtype.starts_with("noicon") || subtype.starts_with("cache.noicon")
            });
            self.client_output.push(Message {
                opcode: OP_TEXT,
                payload: motd_frame(cfg, online, &lines, false, !no_icon),
            });
            if !no_icon {
                self.send(cfg.motd_icon()?);
            }
            self.closing = true;
            return Ok(());
        }
        if message.opcode != OP_BIN {
            return Err("expected binary Eagler packet".into());
        }
        if self.stage == Stage::Play {
            if self.version >= 5 && message.payload.first() == Some(&skins::INJECTED_PACKET) {
                return self.injected_message(&message.payload, store);
            }
            return self.send_backend(&message.payload);
        }
        let mut r = Reader::new(&message.payload);
        let id = r.byte()?;
        match (&self.stage, id) {
            (Stage::Version, 1) => {
                if r.byte()? != 2 {
                    return Err("legacy Eagler handshake is unsupported".into());
                }
                let count = r.short()?;
                if !(1..=16).contains(&count) {
                    return Err("invalid Eagler version count".into());
                }
                for _ in 0..count {
                    let version = r.short()?;
                    if (2..=5).contains(&version) {
                        self.version = self.version.max(version);
                    }
                }
                let count = r.short()?;
                if !(1..=16).contains(&count) {
                    return Err("invalid Minecraft version count".into());
                }
                // Accept the highest client-offered protocol inside the
                // configured range, matching EaglerXServer's selection.
                let (min_protocol, max_protocol) = cfg.minecraft_protocol_range();
                let mut protocol = 0;
                for _ in 0..count {
                    let offered = r.short()?;
                    if (min_protocol..=max_protocol).contains(&offered) {
                        protocol = protocol.max(offered);
                    }
                }
                self.protocol = protocol;
                let _brand = r.ascii()?;
                let _version = r.ascii()?;
                let _auth = r.byte()?;
                let len = r.byte()? as usize;
                let identity = r.take(len)?;
                if !identity.is_empty() {
                    self.handshake_username = Some(
                        String::from_utf8(identity.to_vec())
                            .map_err(|_| "invalid handshake username")?,
                    );
                }
                r.end()?;
                if self.version == 0
                    || self.protocol == 0
                    || (self.protocol > 340 && self.version < 5)
                {
                    self.send(mismatch_packet(cfg));
                    self.closing = true;
                    return Ok(());
                }
                let mut packet = vec![2];
                packet.extend_from_slice(&self.version.to_be_bytes());
                packet.extend_from_slice(&self.protocol.to_be_bytes());
                ascii(&mut packet, "EaglerXPumpkin");
                ascii(&mut packet, env!("CARGO_PKG_VERSION"));
                packet.extend_from_slice(&[0, 0, 0]); // no authentication, zero salt
                if self.version >= 5 {
                    packet.push(0);
                } // nickname selection disabled
                self.send(packet);
                self.stage = Stage::Request;
            }
            (Stage::Request, 4) => {
                let requested_username = r.ascii()?;
                // With nickname selection disabled, use the initial identity,
                // matching EaglerXServer. Legacy clients supply only a nickname.
                let username = self
                    .handshake_username
                    .clone()
                    .unwrap_or(requested_username);
                if username.is_empty()
                    || username.len() > 16
                    || !username
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err("username must contain 1..16 letters, digits or underscores".into());
                }
                let _server = r.ascii()?;
                let len = r.byte()? as usize;
                r.take(len)?; // offline accounts only; no password auth offered
                if self.version >= 4 {
                    let cookie = r.byte()?;
                    let len = r.byte()? as usize;
                    if cookie == 0 && len != 0 {
                        return Err("invalid cookie length".into());
                    }
                    r.take(len)?;
                }
                if self.version >= 5 {
                    let caps = r.int()? as u32;
                    let mut supported = 0u32;
                    for bit in 0..32 {
                        if caps & (1 << bit) != 0 && r.int()? & 1 != 0 {
                            supported |= 1 << bit;
                        }
                    }
                    self.capabilities = supported & optional_capabilities(cfg);
                    let count = r.byte()?;
                    if count > 32 {
                        return Err("too many extended capabilities".into());
                    }
                    for _ in 0..count {
                        r.take(16)?;
                        r.int()?;
                    }
                }
                r.end()?;
                self.username = username;
                self.uuid = Md5::digest(format!("OfflinePlayer:{}", self.username)).into();
                self.uuid[6] = (self.uuid[6] & 15) | 0x30;
                self.uuid[8] = (self.uuid[8] & 63) | 0x80;
                let mut handshake = vec![0];
                mc::varint(&mut handshake, self.protocol as i32);
                mc::string(&mut handshake, &cfg.server_address);
                handshake.extend_from_slice(&cfg.backend_port.to_be_bytes());
                handshake.push(2);
                self.send_backend(&handshake)?;
                let mut login = vec![0];
                mc::string(&mut login, &self.username);
                login_start(&mut login, self.protocol, &self.uuid);
                self.send_backend(&login)?;
                self.connect_requested = true;
                self.stage = Stage::BackendLogin;
            }
            (Stage::Profiles, 7) => {
                let count = if self.version >= 4 { r.byte()? } else { 1 };
                for _ in 0..count {
                    let name = r.ascii()?;
                    let len = r.short()? as usize;
                    let value = r.take(len)?;
                    self.profile_bytes += len;
                    if self.profiles.len() >= 16 || self.profile_bytes > 65536 {
                        return Err("excessive profile data".into());
                    }
                    if !self.profiles.insert(name.clone()) {
                        return Err("duplicate profile data".into());
                    }
                    self.player_skin.apply_profile(&name, value);
                }
                r.end()?;
            }
            (Stage::Profiles, 8) => {
                r.end()?;
                store.register(self.uuid, self.player_skin.clone());
                self.send(vec![9]);
                if self.version >= 5 {
                    self.send(skins::disable_skin_url_lookup());
                }
                self.stage = Stage::Play;
                for packet in std::mem::take(&mut self.stalled) {
                    self.send(packet);
                }
                self.stalled_bytes = 0;
                if self.version >= 5 {
                    let accepted = self.capabilities;
                    if accepted & ui::CAP_PAUSE_MENU != 0
                        && let Some((button, url)) = cfg.pause_menu()
                    {
                        self.send(skins::injected(ui::pause_menu(button, url)));
                    }
                    if accepted & ui::CAP_NOTIFICATION != 0
                        && let Some((title, body, source)) = cfg.notification()
                    {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|elapsed| elapsed.as_secs())
                            .unwrap_or(0);
                        self.send(skins::injected(ui::notification(
                            title,
                            body,
                            source,
                            cfg.notification_hide_after_secs,
                            cfg.notification_expire_after_secs,
                            now,
                        )));
                    }
                }
            }
            _ => return Err("unexpected Eagler handshake packet".into()),
        }
        Ok(())
    }

    fn send_backend(&mut self, packet: &[u8]) -> Result<()> {
        self.backend_output.extend(self.codec.encode(packet)?);
        Ok(())
    }

    /// Handle a V5 `0xEE` injected game message, either a single packet or a
    /// `0xFF` multi-packet batch. Messages this gateway does not implement
    /// (voice, webview, URL lookups) are dropped rather than forwarded, since
    /// the Pumpkin backend would not understand them.
    fn injected_message(&mut self, payload: &[u8], store: &mut SkinStore) -> Result<()> {
        if payload.len() > 65536 {
            return Err("injected message too large".into());
        }
        let body = &payload[1..];
        if body.first() == Some(&0xFF) {
            let mut r = Reader::new(&body[1..]);
            let count = r.int()?;
            if !(0..=16).contains(&count) {
                return Err("invalid injected packet count".into());
            }
            for _ in 0..count {
                let len = r.int()?;
                if len < 0 {
                    return Err("invalid injected packet length".into());
                }
                let packet = r.take(len as usize)?;
                self.injected_packet(packet, store)?;
            }
            r.end()?;
        } else {
            self.injected_packet(body, store)?;
        }
        Ok(())
    }

    fn injected_packet(&mut self, packet: &[u8], store: &mut SkinStore) -> Result<()> {
        let mut r = Reader::new(packet);
        let id = r.int()?;
        if !(0x01..=0x03).contains(&id) {
            return Ok(()); // unsupported injected message: drop it
        }
        let request_id = r.int()?;
        let uuid: [u8; 16] = r.take(16)?.try_into().unwrap();
        r.end()?;
        let response = match id {
            0x01 => store.skin_response(&uuid, request_id),
            0x02 => store.cape_response(&uuid, request_id),
            _ => store.textures_response(&uuid, request_id),
        };
        self.send(skins::injected(response));
        Ok(())
    }

    pub fn backend_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.codec.push(bytes)?;
        // Every packet in this bounded TCP read is processed before another read.
        while let Some(packet) = self.codec.next_packet()? {
            if self.stage == Stage::BackendLogin {
                let mut r = Reader::new(&packet);
                match r.int()? {
                    0 => {
                        let reason = r
                            .text(65536)
                            .unwrap_or_else(|_| "Backend rejected login".into());
                        self.fail(&reason);
                    }
                    1 => self.fail("Pumpkin must use offline mode for Eaglercraft connections"),
                    2 => {
                        if self.protocol >= 735 {
                            self.uuid.copy_from_slice(r.take(16)?);
                        } else {
                            let uuid = r.text(36)?.replace('-', "");
                            if uuid.len() != 32 {
                                return Err("invalid login UUID".into());
                            }
                            for i in 0..16 {
                                self.uuid[i] = u8::from_str_radix(&uuid[i * 2..i * 2 + 2], 16)
                                    .map_err(|_| "invalid UUID")?;
                            }
                        }
                        let username = r.text(64)?;
                        if username.is_empty() || username.len() > 16 || !username.is_ascii() {
                            return Err("invalid backend username".into());
                        }
                        self.username = username;
                        self.player_skin = PlayerSkin::fallback(&self.uuid);
                        if self.protocol >= 764 {
                            self.send_backend(&[3])?;
                        } // login acknowledged
                        let mut allow = vec![5];
                        ascii(&mut allow, &self.username);
                        allow.extend_from_slice(&self.uuid);
                        if self.version >= 5 {
                            let accepted = self.capabilities;
                            mc::varint(&mut allow, accepted as i32);
                            for bit in 0..32 {
                                if accepted & (1 << bit) != 0 {
                                    allow.push(0);
                                }
                            }
                            allow.push(0); // no extended capabilities
                        }
                        self.send(allow);
                        self.stage = Stage::Profiles;
                    }
                    3 => {
                        let threshold = r.int()?;
                        r.end()?;
                        if threshold < 0 || threshold as usize > mc::MAX_PACKET {
                            return Err("invalid compression threshold".into());
                        }
                        self.codec.compression = Some(threshold as usize);
                    }
                    4 => self.fail("Backend requires an unsupported login plugin handshake"),
                    5 => {
                        // optional login cookie request: reply with no cookie
                        let key = r.text(32767)?;
                        let mut reply = vec![4];
                        mc::string(&mut reply, &key);
                        reply.push(0);
                        self.send_backend(&reply)?;
                    }
                    _ => return Err("unexpected backend login packet".into()),
                }
            } else if self.stage == Stage::Profiles {
                self.stalled_bytes += packet.len();
                if self.stalled_bytes > mc::MAX_PACKET {
                    return Err("too much data before Eagler login finish".into());
                }
                self.stalled.push(packet);
            } else if self.stage == Stage::Play {
                self.send(packet);
            }
            if self.closing {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client(session: &mut Session, cfg: &Config, store: &mut SkinStore, bytes: &[u8]) {
        session
            .client_message(
                Message {
                    opcode: OP_BIN,
                    payload: bytes.into(),
                },
                cfg,
                0,
                store,
            )
            .unwrap();
    }
    fn start() -> (Session, Config, SkinStore) {
        let cfg = Config::default();
        let mut session = Session::default();
        let mut store = SkinStore::default();
        client(
            &mut session,
            &cfg,
            &mut store,
            b"\x01\x02\x00\x01\x00\x05\x00\x01\x03\x09\x04Test\x01x\x00\x00",
        );
        assert_eq!(
            &session.client_output.pop().unwrap().payload[..5],
            b"\x02\x00\x05\x03\x09"
        );
        client(
            &mut session,
            &cfg,
            &mut store,
            b"\x04\x04Test\x07default\x00\x00\x00\x00\x00",
        );
        (session, cfg, store)
    }
    #[test]
    fn initial_identity_is_used_when_nickname_selection_is_disabled() {
        for requested in ["", "OtherName"] {
            let mut session = Session::default();
            let cfg = Config::default();
            let mut store = SkinStore::default();
            client(
                &mut session,
                &cfg,
                &mut store,
                b"\x01\x02\x00\x01\x00\x05\x00\x01\x03\x09\x04Test\x01x\x00\x05guest",
            );
            let mut request = vec![4];
            ascii(&mut request, requested);
            ascii(&mut request, "default");
            request.extend_from_slice(&[0, 0, 0, 0, 0]);
            client(&mut session, &cfg, &mut store, &request);
            assert_eq!(session.username, "guest");
            assert!(session.connect_requested);
        }
    }

    #[test]
    fn login_start_matches_each_protocol_generation() {
        let uuid = [0xab; 16];
        let mut username = Vec::new();
        mc::string(&mut username, "Test");
        let with_uuid = |prefix: &[u8]| {
            let mut expected = username.clone();
            expected.extend_from_slice(prefix);
            expected.extend_from_slice(&uuid);
            expected
        };
        let cases = [
            (755, username.clone()),
            (758, username.clone()),
            (759, {
                let mut expected = username.clone();
                expected.push(0); // has_signature_data = false
                expected
            }),
            (760, with_uuid(&[0, 1])), // has_sig = false, has_uuid = true
            (761, with_uuid(&[1])),
            (763, with_uuid(&[1])),
            (764, with_uuid(&[])), // 1.20.2+: unconditional UUID
            (777, with_uuid(&[])),
        ];
        for (protocol, expected) in cases {
            let mut out = username.clone();
            login_start(&mut out, protocol, &uuid);
            assert_eq!(out, expected, "Login Start for protocol {protocol}");
        }
    }

    #[test]
    fn mismatch_packet_advertises_the_configured_protocol_range() {
        // Default config advertises exactly Minecraft 26.3.
        let packet = mismatch_packet(&Config::default());
        assert_eq!(&packet[..9], &[3, 0, 1, 0, 5, 0, 1, 0x03, 0x09]);
        let length = packet[9] as usize;
        let message = std::str::from_utf8(&packet[10..10 + length]).unwrap();
        assert_eq!(
            message,
            "Use a Minecraft 26.3 Eaglercraft client (protocol 777)"
        );
        assert!(packet.len() == 10 + length);

        // A configured range advertises both bounds and actionable guidance.
        let cfg = cfg_range(Some(755), Some(777));
        let packet = mismatch_packet(&cfg);
        assert_eq!(
            &packet[..11],
            &[3, 0, 1, 0, 5, 0, 2, 0x02, 0xf3, 0x03, 0x09]
        );
        let length = packet[11] as usize;
        let message = std::str::from_utf8(&packet[12..12 + length]).unwrap();
        assert_eq!(
            message,
            "Use a Minecraft 1.17-26.3 Eaglercraft client (protocols 755-777)"
        );
        assert!(packet.len() == 12 + length);
    }

    /// A config with an explicit protocol range (or none, for the exact
    /// `minecraft_protocol` default).
    fn cfg_range(min: Option<u16>, max: Option<u16>) -> Config {
        let cfg = Config {
            min_minecraft_protocol: min,
            max_minecraft_protocol: max,
            ..Config::default()
        };
        cfg.validate().unwrap();
        cfg
    }

    /// A V5 handshake packet offering `versions` as the Minecraft protocols.
    fn handshake(versions: &[u16]) -> Vec<u8> {
        handshake_offering(&[5], versions)
    }

    /// An Eagler handshake packet offering `eagler` handshake versions and
    /// `protocols` as the Minecraft protocols.
    fn handshake_offering(eagler: &[u16], protocols: &[u16]) -> Vec<u8> {
        let mut packet = vec![1, 2];
        packet.extend_from_slice(&(eagler.len() as u16).to_be_bytes());
        for version in eagler {
            packet.extend_from_slice(&version.to_be_bytes());
        }
        packet.extend_from_slice(&(protocols.len() as u16).to_be_bytes());
        for protocol in protocols {
            packet.extend_from_slice(&protocol.to_be_bytes());
        }
        ascii(&mut packet, "Test"); // brand
        ascii(&mut packet, "x"); // client version
        packet.push(0); // no authentication
        packet.push(0); // empty initial identity
        packet
    }

    /// A v5 login request for username "Test": no password, optional cookie
    /// bytes, `caps` capability bits marked supported, no extended capabilities.
    fn request_v5(cookie: &[u8], caps: u32) -> Vec<u8> {
        let mut request = vec![4];
        ascii(&mut request, "Test");
        ascii(&mut request, "default");
        request.push(0); // no password
        request.push(u8::from(!cookie.is_empty()));
        request.push(cookie.len() as u8);
        request.extend_from_slice(cookie);
        mc::varint(&mut request, caps as i32);
        for bit in 0..32 {
            if caps & (1 << bit) != 0 {
                mc::varint(&mut request, 1); // capability version 0
            }
        }
        request.push(0); // no extended capabilities
        request
    }

    /// Full packet/state regression for each admitted protocol generation:
    /// negotiation, version-correct Login Start injection, offline login, the
    /// 1.20.2+ Login Acknowledged round trip, compression transition,
    /// configuration stalling and play-packet relay.
    #[test]
    fn full_login_and_compression_flow_matches_each_protocol_generation() {
        for (protocol, threshold) in (755..=777)
            .flat_map(|protocol| [None, Some(1), Some(256)].map(|threshold| (protocol, threshold)))
        {
            let login_acknowledged = protocol >= 764;
            let cfg = cfg_range(Some(protocol), Some(protocol));
            let mut session = Session::default();
            let mut store = SkinStore::default();
            client(&mut session, &cfg, &mut store, &handshake(&[protocol]));
            client(&mut session, &cfg, &mut store, &request_v5(&[], 0));
            assert!(session.connect_requested);
            session.client_output.clear(); // discard the version negotiation reply

            // The backend receives the injected Handshake and Login Start.
            let mut backend = Codec::default();
            backend
                .push(&std::mem::take(&mut session.backend_output))
                .unwrap();
            let handshake_packet = backend.next_packet().unwrap().unwrap();
            let mut r = Reader::new(&handshake_packet);
            assert_eq!(r.byte().unwrap(), 0);
            assert_eq!(r.int().unwrap(), i32::from(protocol));
            r.text(255).unwrap(); // server_address
            assert_eq!(r.short().unwrap(), cfg.backend_port);
            assert_eq!(r.byte().unwrap(), 2); // login next state
            r.end().unwrap();
            let login = backend.next_packet().unwrap().unwrap();
            let mut expected = vec![0];
            mc::string(&mut expected, "Test");
            login_start(&mut expected, protocol, &session.uuid);
            assert_eq!(login, expected, "Login Start for protocol {protocol}");

            // The backend enables compression before the login succeeds, and
            // the 1.20.2+ Login Acknowledged reply uses the same codec.
            if let Some(threshold) = threshold {
                let mut compression = vec![3];
                mc::varint(&mut compression, threshold as i32);
                session
                    .backend_bytes(&backend.encode(&compression).unwrap())
                    .unwrap();
            }
            assert_eq!(session.codec.compression, threshold);
            backend.compression = threshold;

            let mut success = vec![2];
            success.extend_from_slice(&[1; 16]);
            mc::string(&mut success, "Test");
            session
                .backend_bytes(&backend.encode(&success).unwrap())
                .unwrap();
            if login_acknowledged {
                let mut echo = Codec::default();
                echo.compression = threshold;
                echo.push(&std::mem::take(&mut session.backend_output))
                    .unwrap();
                assert_eq!(echo.next_packet().unwrap().unwrap(), [3]);
                assert!(echo.next_packet().unwrap().is_none());
            } else {
                assert!(session.backend_output.is_empty());
            }
            let allow = session.client_output.pop().unwrap().payload;
            assert_eq!(allow[0], 5);
            assert_eq!(&allow[6..22], &[1; 16]); // backend UUID wins
            assert!(session.client_output.is_empty());

            // Backend packets stall until the Eagler login finishes,
            // then flush to the client in order ahead of the play stage.
            // Older protocols enter play directly; newer ones enter configuration.
            let known = if login_acknowledged {
                vec![15, 0]
            } else {
                vec![0x21, 0]
            };
            session
                .backend_bytes(&backend.encode(&known).unwrap())
                .unwrap();
            let profile = vec![7, 0]; // no profile data
            client(&mut session, &cfg, &mut store, &profile);
            client(&mut session, &cfg, &mut store, &[8]);
            let outputs: Vec<Vec<u8>> = session
                .client_output
                .drain(..)
                .map(|message| message.payload)
                .collect();
            assert_eq!(outputs, [vec![9], skins::disable_skin_url_lookup(), known]);
            assert!(session.is_playing());

            // Play packets relay in both directions over the compressed
            // backend leg, one packet body per WebSocket message.
            let keepalive = [0x19, 1, 2, 3, 4, 5, 6, 7, 8];
            session
                .backend_bytes(&backend.encode(&keepalive).unwrap())
                .unwrap();
            assert_eq!(session.client_output.pop().unwrap().payload, keepalive);
            client(&mut session, &cfg, &mut store, &[3]); // opaque serverbound packet
            let mut echo = Codec::default();
            echo.compression = threshold;
            echo.push(&std::mem::take(&mut session.backend_output))
                .unwrap();
            assert_eq!(echo.next_packet().unwrap().unwrap(), [3]);
            assert!(echo.next_packet().unwrap().is_none());
        }
    }

    #[test]
    fn negotiates_the_highest_offered_protocol_inside_the_range() {
        for (min, max, offered, selected) in [
            (Some(755), Some(777), vec![755, 760, 777, 754], 777),
            (Some(755), Some(777), vec![755], 755),
            (Some(755), Some(760), vec![755, 760, 777], 760),
            (None, None, vec![777], 777),
        ] {
            let cfg = cfg_range(min, max);
            let mut session = Session::default();
            let mut store = SkinStore::default();
            client(&mut session, &cfg, &mut store, &handshake(&offered));
            let reply = session.client_output.pop().unwrap().payload;
            assert_eq!(
                &reply[..5],
                &[2, 0, 5, (selected >> 8) as u8, selected as u8],
                "server version packet for {selected}"
            );
            assert_eq!(session.protocol, selected);
        }
    }

    #[test]
    fn login_start_uses_the_selected_protocol_from_the_range() {
        let cfg = cfg_range(Some(755), Some(760));
        let mut session = Session::default();
        let mut store = SkinStore::default();
        client(&mut session, &cfg, &mut store, &handshake(&[755, 760, 777]));
        assert_eq!(session.protocol, 760);
        client(
            &mut session,
            &cfg,
            &mut store,
            b"\x04\x04Test\x07default\x00\x00\x00\x00\x00",
        );
        let mut backend = Codec::default();
        backend
            .push(&std::mem::take(&mut session.backend_output))
            .unwrap();
        let handshake_packet = backend.next_packet().unwrap().unwrap();
        let mut backend_reader = Reader::new(&handshake_packet);
        assert_eq!(backend_reader.byte().unwrap(), 0); // handshake
        assert_eq!(backend_reader.int().unwrap(), 760); // selected protocol
        let login = backend.next_packet().unwrap().unwrap();
        let mut expected = vec![0];
        mc::string(&mut expected, "Test");
        expected.extend_from_slice(&[0, 1]); // has_sig = false, has_uuid = true
        expected.extend_from_slice(&session.uuid);
        assert_eq!(login, expected);
    }

    #[test]
    fn protocol_outside_the_configured_range_receives_the_range_mismatch() {
        let cfg = cfg_range(Some(755), Some(760));
        let mut session = Session::default();
        let mut store = SkinStore::default();
        client(&mut session, &cfg, &mut store, &handshake(&[777]));
        assert!(session.closing);
        let packet = session.client_output.pop().unwrap().payload;
        assert_eq!(packet[0], 3);
        assert_eq!(
            &packet[..11],
            &[3, 0, 1, 0, 5, 0, 2, 0x02, 0xf3, 0x02, 0xf8]
        );
        let length = packet[11] as usize;
        assert_eq!(
            std::str::from_utf8(&packet[12..12 + length]).unwrap(),
            mismatch_message(755, 760)
        );
    }

    #[test]
    fn backend_login_waits_for_client_finish_before_replaying_configuration() {
        let (mut session, cfg, mut store) = start();
        assert!(session.connect_requested);
        let mut backend = Codec::default();
        backend
            .push(&std::mem::take(&mut session.backend_output))
            .unwrap();
        let handshake = backend.next_packet().unwrap().unwrap();
        assert_eq!(&handshake[..3], &[0, 0x89, 6]); // protocol 777 as VarInt
        let login = backend.next_packet().unwrap().unwrap();
        assert_eq!(&login[..6], b"\x00\x04Test");
        assert_eq!(login.len(), 22); // name plus offline UUID
        let mut success = vec![2];
        success.extend_from_slice(&[1; 16]);
        mc::string(&mut success, "Test");
        success.push(0); // properties
        success.extend_from_slice(&[2; 16]); // session id (26.2+)
        let mut wire = backend.encode(&success).unwrap();
        wire.extend(backend.encode(&[15, 0]).unwrap()); // known packs
        session.backend_bytes(&wire).unwrap();
        assert_eq!(session.backend_output, [1, 3]); // login ack precedes Eagler round trip
        let allow = session.client_output.pop().unwrap().payload;
        assert_eq!(allow[0], 5);
        assert_eq!(&allow[6..22], &[1; 16]); // backend UUID wins
        assert!(session.client_output.is_empty());
        client(
            &mut session,
            &cfg,
            &mut store,
            b"\x07\x01\x05brand\x00\x01x",
        );
        client(&mut session, &cfg, &mut store, &[8]);
        assert_eq!(session.client_output[0].payload, [9]);
        assert_eq!(
            session.client_output[1].payload,
            skins::disable_skin_url_lookup()
        );
        assert_eq!(session.client_output[2].payload, [15, 0]);
        assert!(session.is_playing());
        session.backend_output.clear();
        client(&mut session, &cfg, &mut store, &[3]); // finish config remains a Minecraft packet
        assert_eq!(session.backend_output, [1, 3]);
    }
    #[test]
    fn version_mismatch_and_duplicate_profiles_are_rejected() {
        let cfg = Config::default();
        let mut store = SkinStore::default();
        let mut old = Session::default();
        client(
            &mut old,
            &cfg,
            &mut store,
            b"\x01\x02\x00\x01\x00\x05\x00\x01\x01\x54\x01x\x01x\x00\x00",
        );
        assert!(old.closing);
        assert_eq!(old.client_output[0].payload[0], 3);
        let (mut session, cfg, mut store) = start();
        session.stage = Stage::Profiles;
        client(
            &mut session,
            &cfg,
            &mut store,
            b"\x07\x01\x05brand\x00\x01x",
        );
        assert!(
            session
                .client_message(
                    Message {
                        opcode: OP_BIN,
                        payload: b"\x07\x01\x05brand\x00\x01x".to_vec()
                    },
                    &cfg,
                    0,
                    &mut store,
                )
                .is_err()
        );
    }
    #[test]
    fn registers_profile_textures_and_answers_injected_requests() {
        let (mut session, cfg, mut store) = start();
        let mut backend = Codec::default();
        backend
            .push(&std::mem::take(&mut session.backend_output))
            .unwrap();
        let mut success = vec![2];
        success.extend_from_slice(&[7; 16]);
        mc::string(&mut success, "Test");
        success.push(0);
        success.extend_from_slice(&[2; 16]);
        session
            .backend_bytes(&backend.encode(&success).unwrap())
            .unwrap();
        session.client_output.clear(); // allow login

        let mut profile = vec![7, 2];
        profile.push(7);
        profile.extend_from_slice(b"skin_v2");
        profile.extend_from_slice(&5u16.to_be_bytes());
        profile.extend_from_slice(&[1, 0, 0, 0, 4]);
        // cape_v1 preset 2
        profile.push(7);
        profile.extend_from_slice(b"cape_v1");
        profile.extend_from_slice(&5u16.to_be_bytes());
        profile.extend_from_slice(&[1, 0, 0, 0, 2]);
        client(&mut session, &cfg, &mut store, &profile);
        client(&mut session, &cfg, &mut store, &[8]);
        assert!(session.is_playing());
        session.client_output.clear();
        let uuid = [7u8; 16];
        let skin = store.get(&uuid).unwrap();
        assert_eq!(skin.skin, skins::Skin::Preset(4));
        assert_eq!(skin.cape, skins::Cape::Preset(2));

        // Single injected textures request: id 0x03, request 9, UUID.
        let mut request = vec![skins::INJECTED_PACKET, 3, 9];
        request.extend_from_slice(&uuid);
        client(&mut session, &cfg, &mut store, &request);
        let reply = session.client_output.pop().unwrap().payload;
        assert_eq!(reply, skins::injected(vec![5, 9, 4, 2]));
        session.client_output.clear();

        // Multi-packet batch: skin and cape requests in one message.
        let mut skin_request = vec![1, 11];
        skin_request.extend_from_slice(&uuid);
        let mut cape_request = vec![2, 12];
        cape_request.extend_from_slice(&uuid);
        let mut batch = vec![skins::INJECTED_PACKET, 0xFF, 2];
        batch.push(skin_request.len() as u8);
        batch.extend_from_slice(&skin_request);
        batch.push(cape_request.len() as u8);
        batch.extend_from_slice(&cape_request);
        client(&mut session, &cfg, &mut store, &batch);
        let payloads: Vec<Vec<u8>> = session
            .client_output
            .drain(..)
            .map(|message| message.payload)
            .collect();
        assert_eq!(
            payloads,
            [
                skins::injected(vec![1, 11, 4]),
                skins::injected(vec![3, 12, 2])
            ]
        );

        // Unsupported injected messages are dropped, not forwarded backend.
        session.backend_output.clear();
        client(
            &mut session,
            &cfg,
            &mut store,
            &[skins::INJECTED_PACKET, 4, 1, 1, b'x'],
        );
        assert!(session.backend_output.is_empty());
        assert!(session.client_output.is_empty());
    }

    #[test]
    fn static_motd_query_answers_and_closes() {
        let cfg = Config::default();
        let mut session = Session::default();
        let mut store = SkinStore::default();
        session
            .client_message(
                Message {
                    opcode: OP_TEXT,
                    payload: b"Accept: MOTD".to_vec(),
                },
                &cfg,
                2,
                &mut store,
            )
            .unwrap();
        assert!(session.closing);
        assert!(!session.is_animating_motd());
        let text = String::from_utf8(session.client_output[0].payload.clone()).unwrap();
        assert!(text.contains("\"cache\":true"));
        assert!(text.contains("\"online\":2"));
        assert!(text.contains("Minecraft 26.3"));
        let response: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(response["data"]["icon"], true);
        assert_eq!(session.client_output.len(), 2);
        assert_eq!(session.client_output[1].opcode, OP_BIN);
        assert_eq!(session.client_output[1].payload, crate::builtin_motd_icon());
    }

    #[test]
    fn static_motd_query_uses_configured_server_icon() {
        let cfg = Config {
            motd_icon: Some(crate::motd_icon::tests::test_icon_uri()),
            ..Config::default()
        };
        let mut session = Session::default();
        let mut store = SkinStore::default();
        session
            .client_message(
                Message {
                    opcode: OP_TEXT,
                    payload: b"Accept: MOTD".to_vec(),
                },
                &cfg,
                0,
                &mut store,
            )
            .unwrap();
        assert!(session.closing);
        let text = String::from_utf8(session.client_output[0].payload.clone()).unwrap();
        let response: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(response["data"]["icon"], true);
        assert_eq!(session.client_output.len(), 2);
        assert_eq!(session.client_output[1].opcode, OP_BIN);
        assert_eq!(
            session.client_output[1].payload,
            [17, 34, 51, 68].repeat(4096)
        );
    }

    #[test]
    fn animated_motd_cycles_frames_then_closes_after_max_age() {
        let cfg = Config {
            motd_frames: vec!["A\nB".into(), "C\nD".into(), "E".into()],
            motd_interval_ms: 50,
            motd_close_after_ms: 1000,
            ..Config::default()
        };
        cfg.validate().unwrap();
        let mut session = Session::default();
        let mut store = SkinStore::default();
        session
            .client_message(
                Message {
                    opcode: OP_TEXT,
                    payload: b"Accept: MOTD".to_vec(),
                },
                &cfg,
                0,
                &mut store,
            )
            .unwrap();
        assert!(!session.closing);
        assert!(session.is_animating_motd());
        let t0 = Instant::now(); // after the query, so timers are bounded by t0
        let text = String::from_utf8(session.client_output[0].payload.clone()).unwrap();
        assert!(text.contains("\"cache\":false"));
        assert!(text.contains("[\"A\",\"B\"]"));
        assert_eq!(session.client_output.len(), 1);

        // Before the interval elapses nothing new is queued.
        session.motd_tick(&cfg, 0, t0 + Duration::from_millis(20));
        assert_eq!(session.client_output.len(), 1);
        // Frame 1 at the interval boundary.
        session.motd_tick(&cfg, 0, t0 + Duration::from_millis(60));
        let text = String::from_utf8(session.client_output[1].payload.clone()).unwrap();
        assert!(text.contains("[\"C\",\"D\"]"));
        session.motd_tick(&cfg, 0, t0 + Duration::from_millis(80));
        assert_eq!(session.client_output.len(), 2);
        // Frame 2, then wrap around to the first frame.
        session.motd_tick(&cfg, 0, t0 + Duration::from_millis(120));
        assert!(
            String::from_utf8(session.client_output[2].payload.clone())
                .unwrap()
                .contains("[\"E\"]")
        );
        session.motd_tick(&cfg, 0, t0 + Duration::from_millis(190));
        assert!(
            String::from_utf8(session.client_output[3].payload.clone())
                .unwrap()
                .contains("[\"A\",\"B\"]")
        );
        // The max age force-closes the query connection and stops the animation.
        session.motd_tick(&cfg, 0, t0 + Duration::from_millis(1100));
        assert!(session.closing);
        assert!(!session.is_animating_motd());
        session.motd_tick(&cfg, 0, t0 + Duration::from_millis(1200));
        assert_eq!(session.client_output.len(), 4);
    }

    #[test]
    fn advertises_and_sends_configured_pause_menu_and_notification() {
        let cfg = Config {
            pause_menu_button: "Discord".into(),
            pause_menu_url: "https://example.com".into(),
            notification_title: "Welcome".into(),
            notification_body: "Have fun".into(),
            notification_source: "EaglerXPumpkin".into(),
            ..Config::default()
        };
        let mut session = Session::default();
        let mut store = SkinStore::default();
        client(
            &mut session,
            &cfg,
            &mut store,
            b"\x01\x02\x00\x01\x00\x05\x00\x01\x03\x09\x04Test\x01x\x00\x00",
        );
        session.client_output.clear();
        // Capabilities 0x18 (notification + pause menu), both V0.
        client(
            &mut session,
            &cfg,
            &mut store,
            b"\x04\x04Test\x07default\x00\x00\x00\x18\x01\x01\x00",
        );
        assert!(session.client_output.is_empty());

        let mut backend = Codec::default();
        backend
            .push(&std::mem::take(&mut session.backend_output))
            .unwrap();
        backend.next_packet().unwrap().unwrap(); // handshake
        backend.next_packet().unwrap().unwrap(); // login
        let mut success = vec![2];
        success.extend_from_slice(&[7; 16]);
        mc::string(&mut success, "Test");
        success.push(0);
        success.extend_from_slice(&[2; 16]);
        session
            .backend_bytes(&backend.encode(&success).unwrap())
            .unwrap();
        let allow = session.client_output.pop().unwrap().payload;
        assert_eq!(&allow[22..], &[0x18, 0, 0, 0]);

        session.client_output.clear();
        client(&mut session, &cfg, &mut store, &[8]);
        let payloads: Vec<Vec<u8>> = session
            .client_output
            .drain(..)
            .map(|message| message.payload)
            .collect();
        assert_eq!(payloads.len(), 4);
        assert_eq!(payloads[0], [9]);
        assert_eq!(payloads[1][..2], [skins::INJECTED_PACKET, 0x21]);
        assert_eq!(payloads[2][..2], [skins::INJECTED_PACKET, 0x18]);
        assert_eq!(payloads[3][..2], [skins::INJECTED_PACKET, 0x1F]);

        let mut r = mc::Reader::new(&payloads[2][2..]);
        assert_eq!(r.byte().unwrap(), 1); // external URL mode
        assert_eq!(r.text(127).unwrap(), "Discord");
        assert_eq!(r.short().unwrap(), 19);
        assert_eq!(r.take(19).unwrap(), b"https://example.com");
        assert_eq!(r.int().unwrap(), 0); // no image mappings
        r.end().unwrap();

        let mut r = mc::Reader::new(&payloads[3][2..]);
        assert_eq!(r.take(16).unwrap().len(), 16);
        assert_eq!(r.text(32767).unwrap(), "{\"text\":\"Have fun\"}");
        assert_eq!(r.text(255).unwrap(), "{\"text\":\"Welcome\"}");
        assert_eq!(r.text(255).unwrap(), "{\"text\":\"EaglerXPumpkin\"}");
    }

    /// A session whose backend login has succeeded, ready to receive Eagler
    /// profile data, with a codec wired to its backend stream.
    fn profiles_session() -> (Session, Config, SkinStore, Codec) {
        let (mut session, cfg, store) = start();
        let mut backend = Codec::default();
        backend
            .push(&std::mem::take(&mut session.backend_output))
            .unwrap();
        let mut success = vec![2];
        success.extend_from_slice(&[7; 16]);
        mc::string(&mut success, "Test");
        session
            .backend_bytes(&backend.encode(&success).unwrap())
            .unwrap();
        session.client_output.clear();
        (session, cfg, store, backend)
    }

    /// Malformed requests and backend failure paths reject cleanly instead of
    /// reaching the backend or corrupting the session.
    #[test]
    fn malformed_requests_and_backend_failures_reject_cleanly() {
        let mut store = SkinStore::default();

        // Invalid usernames fail before any backend connection is requested.
        for bad in ["", "has space", "bad!chars", "way-too-long-name"] {
            let cfg = Config::default();
            let mut session = Session::default();
            client(&mut session, &cfg, &mut store, &handshake(&[777]));
            let mut request = vec![4];
            ascii(&mut request, bad);
            ascii(&mut request, "default");
            request.extend_from_slice(&[0, 0, 0, 0, 0]);
            let error = session
                .client_message(
                    Message {
                        opcode: OP_BIN,
                        payload: request,
                    },
                    &cfg,
                    0,
                    &mut store,
                )
                .unwrap_err();
            assert_eq!(
                error,
                "username must contain 1..16 letters, digits or underscores"
            );
            assert!(!session.connect_requested);
        }

        let cfg = Config::default();
        // A v5 cookie flag of zero with a nonzero length is rejected.
        let mut session = Session::default();
        client(&mut session, &cfg, &mut store, &handshake(&[777]));
        let error = session
            .client_message(
                Message {
                    opcode: OP_BIN,
                    payload: b"\x04\x04Test\x07default\x00\x00\x05".to_vec(),
                },
                &cfg,
                0,
                &mut store,
            )
            .unwrap_err();
        assert_eq!(error, "invalid cookie length");
        assert!(!session.connect_requested);

        // A cookie carrying data is accepted and the login still proceeds.
        let mut session = Session::default();
        client(&mut session, &cfg, &mut store, &handshake(&[777]));
        client(&mut session, &cfg, &mut store, &request_v5(&[7], 0));
        assert!(session.connect_requested);

        // An out-of-order login finish during the request stage is rejected.
        let mut session = Session::default();
        client(&mut session, &cfg, &mut store, &handshake(&[777]));
        let error = session
            .client_message(
                Message {
                    opcode: OP_BIN,
                    payload: vec![8],
                },
                &cfg,
                0,
                &mut store,
            )
            .unwrap_err();
        assert_eq!(error, "unexpected Eagler handshake packet");

        // A legacy Eagler v4 handshake cannot negotiate a modern protocol.
        let cfg = cfg_range(Some(755), Some(777));
        let mut session = Session::default();
        client(
            &mut session,
            &cfg,
            &mut store,
            &handshake_offering(&[4], &[755]),
        );
        assert!(session.closing);
        assert_eq!(session.client_output[0].payload[0], 3); // mismatch packet

        // Excessive profile data is rejected on entry count and total bytes.
        let (mut session, cfg, mut store, _backend) = profiles_session();
        let mut profile = vec![7, 17];
        for index in 0..17u8 {
            profile.push(1); // name length
            profile.push(b'a' + index);
            profile.extend_from_slice(&[0, 0]); // empty value
        }
        let error = session
            .client_message(
                Message {
                    opcode: OP_BIN,
                    payload: profile,
                },
                &cfg,
                0,
                &mut store,
            )
            .unwrap_err();
        assert_eq!(error, "excessive profile data");

        let (mut session, cfg, mut store, _backend) = profiles_session();
        let mut profile = vec![7, 2, 0];
        profile.extend_from_slice(&65535u16.to_be_bytes());
        profile.extend(vec![0u8; 65535]);
        profile.push(0);
        profile.extend_from_slice(&2u16.to_be_bytes());
        profile.extend_from_slice(&[0, 0]);
        let error = session
            .client_message(
                Message {
                    opcode: OP_BIN,
                    payload: profile,
                },
                &cfg,
                0,
                &mut store,
            )
            .unwrap_err();
        assert_eq!(error, "excessive profile data");

        // An encryption request means the backend is not in offline mode.
        let (mut session, _cfg, _store) = start();
        let mut backend = Codec::default();
        backend
            .push(&std::mem::take(&mut session.backend_output))
            .unwrap();
        session
            .backend_bytes(&backend.encode(&[1]).unwrap())
            .unwrap();
        assert!(session.closing);
        let disconnect = session.client_output[0].payload.clone();
        assert_eq!(&disconnect[..2], &[0xff, 8]);
        assert!(String::from_utf8_lossy(&disconnect[4..]).contains("offline mode"));

        // A backend disconnect propagates its reason to the client.
        let (mut session, _cfg, _store) = start();
        let mut backend = Codec::default();
        backend
            .push(&std::mem::take(&mut session.backend_output))
            .unwrap();
        let mut packet = vec![0u8]; // disconnect
        mc::string(&mut packet, "No reason");
        session
            .backend_bytes(&backend.encode(&packet).unwrap())
            .unwrap();
        assert!(session.closing);
        let disconnect = session.client_output[0].payload.clone();
        assert_eq!(&disconnect[..2], &[0xff, 8]);
        assert!(String::from_utf8_lossy(&disconnect[4..]).contains("No reason"));

        // An unexpected backend login packet id fails the session.
        let (mut session, _cfg, _store) = start();
        let mut backend = Codec::default();
        backend
            .push(&std::mem::take(&mut session.backend_output))
            .unwrap();
        let error = session
            .backend_bytes(&backend.encode(&[9]).unwrap())
            .unwrap_err();
        assert_eq!(error, "unexpected backend login packet");
    }
}
