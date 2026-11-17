// Eagler handshake ported from EaglerXServer HandshakerV2–V5 and
// WebSocketEaglerInitialHandler. Copyright (c) 2022–2025 lax1dude.
// Distributed under the license in the repository root.
use crate::{
    config::Config,
    minecraft::{self as mc, Codec, Reader, Result},
};
use eagler_ws::{Message, OP_BIN, OP_TEXT};
use md5::{Digest, Md5};
use std::collections::HashSet;

#[derive(Debug, PartialEq, Eq)]
enum Stage {
    Version,
    Request,
    BackendLogin,
    Profiles,
    Play,
}

pub struct Session {
    stage: Stage,
    version: u16,
    protocol: u16,
    username: String,
    uuid: [u8; 16],
    codec: Codec,
    profiles: HashSet<String>,
    profile_bytes: usize,
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
            uuid: [0; 16],
            codec: Codec::default(),
            profiles: HashSet::new(),
            profile_bytes: 0,
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

    pub fn client_message(&mut self, message: Message, cfg: &Config, online: usize) -> Result<()> {
        if self.closing {
            return Ok(());
        }
        if message.opcode == OP_TEXT && self.stage == Stage::Version {
            let request = std::str::from_utf8(&message.payload).map_err(|_| "invalid query")?;
            if !request.to_ascii_lowercase().starts_with("accept: motd") {
                return Err("unsupported query".into());
            }
            let response = serde_json::json!({
                "name": "EaglerXPumkin", "brand": "EaglerXPumkin", "vers": env!("CARGO_PKG_VERSION"),
                "cracked": true, "secure": false, "type": "motd", "time": 0,
                "data": { "motd": cfg.motd.lines().take(2).collect::<Vec<_>>(), "cache": true,
                    "icon": false, "online": online, "max": cfg.max_connections, "players": [] }
            });
            self.client_output.push(Message {
                opcode: OP_TEXT,
                payload: response.to_string().into_bytes(),
            });
            self.closing = true;
            return Ok(());
        }
        if message.opcode != OP_BIN {
            return Err("expected binary Eagler packet".into());
        }
        if self.stage == Stage::Play {
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
                for _ in 0..count {
                    if r.short()? == cfg.minecraft_protocol {
                        self.protocol = cfg.minecraft_protocol;
                    }
                }
                let _brand = r.ascii()?;
                let _version = r.ascii()?;
                let _auth = r.byte()?;
                let len = r.byte()? as usize;
                r.take(len)?;
                r.end()?;
                if self.version == 0
                    || self.protocol == 0
                    || (self.protocol > 340 && self.version < 5)
                {
                    let mut packet = vec![3, 0, 1, 0, 5, 0, 1];
                    packet.extend_from_slice(&cfg.minecraft_protocol.to_be_bytes());
                    ascii(&mut packet, "Use a Minecraft 26.3 Eaglercraft client");
                    self.send(packet);
                    self.closing = true;
                    return Ok(());
                }
                let mut packet = vec![2];
                packet.extend_from_slice(&self.version.to_be_bytes());
                packet.extend_from_slice(&self.protocol.to_be_bytes());
                ascii(&mut packet, "EaglerXPumkin");
                ascii(&mut packet, env!("CARGO_PKG_VERSION"));
                packet.extend_from_slice(&[0, 0, 0]); // no authentication, zero salt
                if self.version >= 5 {
                    packet.push(0);
                } // nickname selection disabled
                self.send(packet);
                self.stage = Stage::Request;
            }
            (Stage::Request, 4) => {
                let username = r.ascii()?;
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
                    for _ in 0..caps.count_ones() {
                        r.int()?;
                    }
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
                if (759..=760).contains(&self.protocol) {
                    login.push(0);
                }
                if self.protocol >= 760 {
                    login.extend_from_slice(&self.uuid);
                }
                self.send_backend(&login)?;
                self.connect_requested = true;
                self.stage = Stage::BackendLogin;
            }
            (Stage::Profiles, 7) => {
                let count = if self.version >= 4 { r.byte()? } else { 1 };
                for _ in 0..count {
                    let name = r.ascii()?;
                    let len = r.short()? as usize;
                    r.take(len)?;
                    self.profile_bytes += len;
                    if self.profiles.len() >= 16 || self.profile_bytes > 65536 {
                        return Err("excessive profile data".into());
                    }
                    if !self.profiles.insert(name) {
                        return Err("duplicate profile data".into());
                    }
                }
                r.end()?;
            }
            (Stage::Profiles, 8) => {
                r.end()?;
                self.send(vec![9]);
                self.stage = Stage::Play;
                for packet in std::mem::take(&mut self.stalled) {
                    self.send(packet);
                }
                self.stalled_bytes = 0;
            }
            _ => return Err("unexpected Eagler handshake packet".into()),
        }
        Ok(())
    }

    fn send_backend(&mut self, packet: &[u8]) -> Result<()> {
        self.backend_output.extend(self.codec.encode(packet)?);
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
                        if self.protocol >= 764 {
                            self.send_backend(&[3])?;
                        } // login acknowledged
                        let mut allow = vec![5];
                        ascii(&mut allow, &self.username);
                        allow.extend_from_slice(&self.uuid);
                        if self.version >= 5 {
                            allow.extend_from_slice(&[0, 0]);
                        } // no optional capabilities
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
    fn client(session: &mut Session, cfg: &Config, bytes: &[u8]) {
        session
            .client_message(
                Message {
                    opcode: OP_BIN,
                    payload: bytes.into(),
                },
                cfg,
                0,
            )
            .unwrap();
    }
    fn start() -> (Session, Config) {
        let cfg = Config::default();
        let mut session = Session::default();
        client(
            &mut session,
            &cfg,
            b"\x01\x02\x00\x01\x00\x05\x00\x01\x03\x09\x04Test\x01x\x00\x00",
        );
        assert_eq!(
            &session.client_output.pop().unwrap().payload[..5],
            b"\x02\x00\x05\x03\x09"
        );
        client(
            &mut session,
            &cfg,
            b"\x04\x04Test\x07default\x00\x00\x00\x00\x00",
        );
        (session, cfg)
    }
    #[test]
    fn backend_login_waits_for_client_finish_before_replaying_configuration() {
        let (mut session, cfg) = start();
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
        client(&mut session, &cfg, b"\x07\x01\x05brand\x00\x01x");
        client(&mut session, &cfg, &[8]);
        assert_eq!(session.client_output[0].payload, [9]);
        assert_eq!(session.client_output[1].payload, [15, 0]);
        assert!(session.is_playing());
        session.backend_output.clear();
        client(&mut session, &cfg, &[3]); // finish config remains a Minecraft packet
        assert_eq!(session.backend_output, [1, 3]);
    }
    #[test]
    fn version_mismatch_and_duplicate_profiles_are_rejected() {
        let cfg = Config::default();
        let mut old = Session::default();
        client(
            &mut old,
            &cfg,
            b"\x01\x02\x00\x01\x00\x05\x00\x01\x01\x54\x01x\x01x\x00\x00",
        );
        assert!(old.closing);
        assert_eq!(old.client_output[0].payload[0], 3);
        let (mut session, cfg) = start();
        session.stage = Stage::Profiles;
        client(&mut session, &cfg, b"\x07\x01\x05brand\x00\x01x");
        assert!(
            session
                .client_message(
                    Message {
                        opcode: OP_BIN,
                        payload: b"\x07\x01\x05brand\x00\x01x".to_vec()
                    },
                    &cfg,
                    0
                )
                .is_err()
        );
    }
}
