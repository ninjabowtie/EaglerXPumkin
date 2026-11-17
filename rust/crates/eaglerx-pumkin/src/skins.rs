//! Eagler V5 skin profile data and injected skin responses.
//!
//! Ported from EaglerXServer's `SkinHandshake` and `SkinPacketVersionCache`
//! (Copyright (c) 2022-2025 lax1dude). Distributed under the license in the
//! repository root.
//!
//! V5 clients send their skin and cape during the Eagler profile-data step and
//! later request other players' textures with injected `0xEE` game messages.
//! This gateway only needs the data for players that are currently connected,
//! so the store is in memory and is not persisted.
use crate::minecraft as mc;
use std::collections::HashMap;

pub const SKIN_BYTES: usize = 12288;
pub const CAPE_BYTES: usize = 1173;
pub const INJECTED_PACKET: u8 = 0xEE;
const MAX_PLAYERS: usize = 1024;
const MAX_PRESET: i32 = 31;

const PACKET_MY_SKIN_PRESET: u8 = 0x01;
const PACKET_MY_SKIN_CUSTOM: u8 = 0x02;
const PACKET_MY_CAPE_PRESET: u8 = 0x01;
const PACKET_MY_CAPE_CUSTOM: u8 = 0x02;

const PACKET_OTHER_SKIN_PRESET: u8 = 0x01;
const PACKET_OTHER_SKIN_CUSTOM: u8 = 0x02;
const PACKET_OTHER_CAPE_PRESET: u8 = 0x03;
const PACKET_OTHER_CAPE_CUSTOM: u8 = 0x04;
const PACKET_OTHER_TEXTURES: u8 = 0x05;
const PACKET_CLIENT_STATE_FLAG: u8 = 0x21;

const DISABLE_SKIN_URL_LOOKUP: (u64, u64) = (0xC41D641BE2DA4094, 0xB1B2DFF2E9D08180);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skin {
    Preset(i32),
    Custom { model: u8, pixels: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cape {
    Preset(i32),
    Custom(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerSkin {
    pub skin: Skin,
    pub cape: Cape,
}

impl Default for PlayerSkin {
    fn default() -> Self {
        Self {
            skin: Skin::Preset(0),
            cape: Cape::Preset(0),
        }
    }
}

impl PlayerSkin {
    /// Deterministic Steve/Alex fallback, matching EaglerXServer.
    pub fn fallback(uuid: &[u8; 16]) -> Self {
        Self {
            skin: Skin::Preset(fallback_preset(uuid)),
            cape: Cape::Preset(0),
        }
    }

    pub fn apply_profile(&mut self, key: &str, value: &[u8]) {
        match key {
            "skin_v2" => self.apply_skin_v2(value),
            "skin_v1" => self.apply_skin_v1(value),
            "cape_v1" => self.apply_cape(value),
            _ => {}
        }
    }

    fn apply_skin_v2(&mut self, data: &[u8]) {
        match data.first().copied() {
            Some(PACKET_MY_SKIN_PRESET) => {
                if let Some(preset) = preset_id(data) {
                    self.skin = Skin::Preset(preset);
                }
            }
            Some(PACKET_MY_SKIN_CUSTOM) if data.len() == 2 + SKIN_BYTES => {
                self.skin = Skin::Custom {
                    model: custom_model(data[1]),
                    pixels: data[2..].to_vec(),
                };
            }
            _ => {}
        }
    }

    fn apply_skin_v1(&mut self, data: &[u8]) {
        match data.first().copied() {
            Some(PACKET_MY_SKIN_PRESET) => {
                if let Some(preset) = preset_id(data) {
                    self.skin = Skin::Preset(preset);
                }
            }
            Some(PACKET_MY_SKIN_CUSTOM) if data.len() == 2 + 16384 => {
                self.skin = Skin::Custom {
                    model: custom_model(data[1]),
                    pixels: convert_v3_to_v4(&data[2..]),
                };
            }
            _ => {}
        }
    }

    fn apply_cape(&mut self, data: &[u8]) {
        match data.first().copied() {
            Some(PACKET_MY_CAPE_PRESET) => {
                if let Some(preset) = preset_id(data) {
                    self.cape = Cape::Preset(preset);
                }
            }
            Some(PACKET_MY_CAPE_CUSTOM) if data.len() == 1 + CAPE_BYTES => {
                self.cape = Cape::Custom(data[1..].to_vec());
            }
            _ => {}
        }
    }
}

fn preset_id(data: &[u8]) -> Option<i32> {
    if data.len() != 5 || data[0] != PACKET_MY_SKIN_PRESET {
        return None;
    }
    let id = (((data[1] & 0x7F) as i32) << 24)
        | ((data[2] as i32) << 16)
        | ((data[3] as i32) << 8)
        | data[4] as i32;
    (0..=MAX_PRESET).contains(&id).then_some(id)
}

fn custom_model(raw: u8) -> u8 {
    (raw & 0x7F).min(0x7E) | 0x80
}

fn fallback_preset(uuid: &[u8; 16]) -> i32 {
    let most = u64::from_be_bytes(uuid[..8].try_into().unwrap());
    let least = u64::from_be_bytes(uuid[8..].try_into().unwrap());
    let mixed = most ^ least;
    (((mixed as u32) ^ ((mixed >> 32) as u32)) & 1) as i32
}

/// Eagler's 64x64 ABGR pixels to the packed RGB + alpha-bit format.
fn convert_v3_to_v4(v3: &[u8]) -> Vec<u8> {
    let mut v4 = vec![0u8; SKIN_BYTES];
    for i in 0..4096 {
        let j = i << 2;
        let k = i * 3;
        v4[k] = v3[j + 1];
        v4[k + 1] = v3[j + 2];
        v4[k + 2] = (v3[j + 3] >> 1) | (v3[j] & 0x80);
    }
    v4
}

/// Connected players' textures, keyed by offline UUID.
#[derive(Default)]
pub struct SkinStore {
    players: HashMap<[u8; 16], PlayerSkin>,
}

impl SkinStore {
    pub fn register(&mut self, uuid: [u8; 16], skin: PlayerSkin) {
        if self.players.len() >= MAX_PLAYERS && !self.players.contains_key(&uuid) {
            // Live connections are bounded well below this; overflow only means
            // stale entries accumulated across reconnects. Evict any one.
            if let Some(stale) = self.players.keys().next().copied() {
                self.players.remove(&stale);
            }
        }
        self.players.insert(uuid, skin);
    }

    pub fn get(&self, uuid: &[u8; 16]) -> Option<&PlayerSkin> {
        self.players.get(uuid)
    }

    pub fn remove(&mut self, uuid: &[u8; 16]) {
        self.players.remove(uuid);
    }

    pub fn skin_response(&self, uuid: &[u8; 16], request_id: i32) -> Vec<u8> {
        match self.get(uuid) {
            Some(player) => encode_skin(request_id, &player.skin),
            None => encode_skin(request_id, &Skin::Preset(fallback_preset(uuid))),
        }
    }

    pub fn cape_response(&self, uuid: &[u8; 16], request_id: i32) -> Vec<u8> {
        match self.get(uuid) {
            Some(player) => encode_cape(request_id, &player.cape),
            None => encode_cape(request_id, &Cape::Preset(0)),
        }
    }

    pub fn textures_response(&self, uuid: &[u8; 16], request_id: i32) -> Vec<u8> {
        let fallback = PlayerSkin::fallback(uuid);
        let player = self.get(uuid).unwrap_or(&fallback);
        let (skin_id, skin_data) = match &player.skin {
            Skin::Preset(id) => (*id, None),
            Skin::Custom { model, pixels } => (-(*model as i32) - 1, Some(pixels.as_slice())),
        };
        let (cape_id, cape_data) = match &player.cape {
            Cape::Preset(id) => (*id, None),
            Cape::Custom(pixels) => (-1, Some(pixels.as_slice())),
        };
        let mut out = vec![PACKET_OTHER_TEXTURES];
        mc::varint(&mut out, request_id);
        mc::varint(&mut out, skin_id);
        mc::varint(&mut out, cape_id);
        if let Some(data) = skin_data {
            out.extend_from_slice(data);
        }
        if let Some(data) = cape_data {
            out.extend_from_slice(data);
        }
        out
    }
}

fn encode_skin(request_id: i32, skin: &Skin) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + SKIN_BYTES);
    match skin {
        Skin::Preset(id) => {
            out.push(PACKET_OTHER_SKIN_PRESET);
            mc::varint(&mut out, request_id);
            mc::varint(&mut out, *id);
        }
        Skin::Custom { model, pixels } => {
            out.push(PACKET_OTHER_SKIN_CUSTOM);
            mc::varint(&mut out, request_id);
            out.push(*model);
            out.extend_from_slice(pixels);
        }
    }
    out
}

fn encode_cape(request_id: i32, cape: &Cape) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + CAPE_BYTES);
    match cape {
        Cape::Preset(id) => {
            out.push(PACKET_OTHER_CAPE_PRESET);
            mc::varint(&mut out, request_id);
            mc::varint(&mut out, *id);
        }
        Cape::Custom(pixels) => {
            out.push(PACKET_OTHER_CAPE_CUSTOM);
            mc::varint(&mut out, request_id);
            out.extend_from_slice(pixels);
        }
    }
    out
}

/// Minecraft packet body announcing that URL skin lookups are unavailable.
pub fn disable_skin_url_lookup() -> Vec<u8> {
    let mut out = vec![INJECTED_PACKET, PACKET_CLIENT_STATE_FLAG];
    out.extend_from_slice(&DISABLE_SKIN_URL_LOOKUP.0.to_be_bytes());
    out.extend_from_slice(&DISABLE_SKIN_URL_LOOKUP.1.to_be_bytes());
    mc::varint(&mut out, 1);
    out
}

/// Wrap a V5 game message in the injected packet marker.
pub fn injected(mut payload: Vec<u8>) -> Vec<u8> {
    payload.insert(0, INJECTED_PACKET);
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid() -> [u8; 16] {
        let mut uuid = [0u8; 16];
        uuid[15] = 1;
        uuid
    }

    #[test]
    fn parses_preset_and_custom_profile_data() {
        let mut player = PlayerSkin::default();
        player.apply_profile("skin_v2", &[1, 0, 0, 0, 7]);
        assert_eq!(player.skin, Skin::Preset(7));
        player.apply_profile("cape_v1", &[1, 0, 0, 0, 3]);
        assert_eq!(player.cape, Cape::Preset(3));

        let mut value = vec![2, 0x81];
        value.extend((0..SKIN_BYTES).map(|i| (i % 251) as u8));
        player.apply_profile("skin_v2", &value);
        match &player.skin {
            Skin::Custom { model, pixels } => {
                assert_eq!(*model, 0x81);
                assert_eq!(pixels.len(), SKIN_BYTES);
            }
            other => panic!("unexpected skin {other:?}"),
        }

        let mut cape = vec![2];
        cape.extend((0..CAPE_BYTES).map(|i| (i % 253) as u8));
        player.apply_profile("cape_v1", &cape);
        assert!(matches!(player.cape, Cape::Custom(ref p) if p.len() == CAPE_BYTES));
    }

    #[test]
    fn malformed_profile_data_keeps_the_previous_texture() {
        let mut player = PlayerSkin::default();
        player.apply_profile("skin_v2", &[1, 0, 0, 0, 4]);
        for value in [
            vec![],
            vec![1],
            vec![1, 0, 0, 0],
            vec![1, 0, 0, 0, 32],
            vec![2, 0x80],
            vec![3, 0, 0, 0, 1],
        ] {
            player.apply_profile("skin_v2", &value);
        }
        assert_eq!(player.skin, Skin::Preset(4));
    }

    #[test]
    fn converts_v3_pixels_to_eagler_texture() {
        let mut v3 = vec![0u8; 16384];
        v3[0] = 0xFF; // alpha bit
        v3[1] = 0x10;
        v3[2] = 0x20;
        v3[3] = 0x30;
        let v4 = convert_v3_to_v4(&v3);
        assert_eq!(&v4[..3], &[0x10, 0x20, 0x98]);
    }

    #[test]
    fn encodes_skin_cape_and_combined_textures() {
        let mut store = SkinStore::default();
        let mut player = PlayerSkin::default();
        player.apply_profile("skin_v2", &[1, 0, 0, 0, 6]);
        player.apply_profile("cape_v1", &[1, 0, 0, 0, 2]);
        store.register(uuid(), player);
        assert_eq!(store.skin_response(&uuid(), 9), [1, 9, 6]);
        assert_eq!(store.cape_response(&uuid(), 9), [3, 9, 2]);
        assert_eq!(store.textures_response(&uuid(), 9), [5, 9, 6, 2]);

        let mut custom = PlayerSkin::default();
        let mut skin = vec![2, 0x80];
        skin.extend((0..SKIN_BYTES).map(|i| (i % 251) as u8));
        custom.apply_profile("skin_v2", &skin);
        let mut cape = vec![2];
        cape.extend((0..CAPE_BYTES).map(|i| (i % 253) as u8));
        custom.apply_profile("cape_v1", &cape);
        store.register(uuid(), custom);

        let response = store.textures_response(&uuid(), 9);
        let mut r = mc::Reader::new(&response);
        assert_eq!(r.byte().unwrap(), 5);
        assert_eq!(r.int().unwrap(), 9);
        assert_eq!(r.int().unwrap(), -129);
        assert_eq!(r.int().unwrap(), -1);
        assert_eq!(r.take(SKIN_BYTES).unwrap().len(), SKIN_BYTES);
        assert_eq!(r.take(CAPE_BYTES).unwrap().len(), CAPE_BYTES);
        r.end().unwrap();

        let missing = store.skin_response(&[0xAB; 16], 3);
        assert!(missing == [1, 3, 0] || missing == [1, 3, 1]);
    }
}
