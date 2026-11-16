//! Minecraft TCP framing and compression. Eagler messages carry packet bodies,
//! whereas the backend expects VarInt lengths and negotiated zlib compression.
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use std::io::{Read, Write};

pub const MAX_PACKET: usize = 8 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, String>;

pub fn varint(out: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        out.push(byte | if value == 0 { 0 } else { 128 });
        if value == 0 {
            break;
        }
    }
}

pub fn read_varint(bytes: &[u8]) -> Result<Option<(i32, usize)>> {
    let mut result = 0u32;
    for (i, &byte) in bytes.iter().take(5).enumerate() {
        if i == 4 && byte & 0xf0 != 0 {
            return Err("VarInt overflow".into());
        }
        result |= ((byte & 127) as u32) << (i * 7);
        if byte & 128 == 0 {
            return Ok(Some((result as i32, i + 1)));
        }
    }
    if bytes.len() >= 5 {
        Err("VarInt too long".into())
    } else {
        Ok(None)
    }
}

pub fn string(out: &mut Vec<u8>, value: &str) {
    varint(out, value.len() as i32);
    out.extend_from_slice(value.as_bytes());
}

pub struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let value = self.bytes.get(..count).ok_or("truncated packet")?;
        self.bytes = &self.bytes[count..];
        Ok(value)
    }
    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn short(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn int(&mut self) -> Result<i32> {
        let (value, len) = read_varint(self.bytes)?.ok_or("truncated VarInt")?;
        self.take(len)?;
        Ok(value)
    }
    pub fn text(&mut self, max: usize) -> Result<String> {
        let len = self.int()?;
        if len < 0 || len as usize > max {
            return Err("string too long".into());
        }
        String::from_utf8(self.take(len as usize)?.to_vec()).map_err(|_| "invalid UTF-8".into())
    }
    pub fn ascii(&mut self) -> Result<String> {
        let len = self.byte()? as usize;
        let value = self.take(len)?;
        if !value.is_ascii() {
            return Err("non-ASCII handshake field".into());
        }
        Ok(String::from_utf8(value.to_vec()).unwrap())
    }
    pub fn end(&self) -> Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err("trailing packet data".into())
        }
    }
}

#[derive(Default)]
pub struct Codec {
    input: Vec<u8>,
    pub compression: Option<usize>,
}
impl Codec {
    pub fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if self.input.len() + bytes.len() > MAX_PACKET + 65536 {
            return Err("backend receive buffer too large".into());
        }
        self.input.extend_from_slice(bytes);
        Ok(())
    }
    pub fn next_packet(&mut self) -> Result<Option<Vec<u8>>> {
        let Some((len, offset)) = read_varint(&self.input)? else {
            return Ok(None);
        };
        if len <= 0 || len as usize > MAX_PACKET {
            return Err("invalid backend packet length".into());
        }
        let end = offset + len as usize;
        if self.input.len() < end {
            return Ok(None);
        }
        let data = self.input[offset..end].to_vec();
        self.input.drain(..end);
        let Some(threshold) = self.compression else {
            return Ok(Some(data));
        };
        let (size, offset) = read_varint(&data)?.ok_or("missing uncompressed length")?;
        if size == 0 {
            if data.len() - offset >= threshold && threshold != 0 {
                return Err("uncompressed packet exceeds threshold".into());
            }
            return Ok(Some(data[offset..].to_vec()));
        }
        if size < 0 || size as usize > MAX_PACKET || (size as usize) < threshold {
            return Err("invalid uncompressed length".into());
        }
        let mut decoder = ZlibDecoder::new(&data[offset..]);
        let mut out = Vec::new();
        (&mut decoder)
            .take(size as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|e| e.to_string())?;
        if out.len() != size as usize || decoder.total_in() as usize != data.len() - offset {
            return Err("compressed packet length mismatch".into());
        }
        Ok(Some(out))
    }
    pub fn encode(&self, packet: &[u8]) -> Result<Vec<u8>> {
        if packet.is_empty() || packet.len() > MAX_PACKET {
            return Err("invalid Minecraft packet size".into());
        }
        let mut data = Vec::new();
        if let Some(threshold) = self.compression {
            if packet.len() >= threshold {
                varint(&mut data, packet.len() as i32);
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
                encoder.write_all(packet).map_err(|e| e.to_string())?;
                data.extend(encoder.finish().map_err(|e| e.to_string())?);
            } else {
                data.push(0);
                data.extend_from_slice(packet);
            }
        } else {
            data.extend_from_slice(packet);
        }
        let mut framed = Vec::new();
        varint(&mut framed, data.len() as i32);
        framed.extend(data);
        Ok(framed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_tcp_splits_and_compression_transition() {
        // Fixed wire examples: packet [0x03, 0x01] enables compression at 1 byte;
        // the following zlib stream is generated independently in Python/zlib.
        let mut codec = Codec::default();
        codec.push(&[2]).unwrap();
        assert!(codec.next_packet().unwrap().is_none());
        codec
            .push(&[
                3, 1, 12, 3, 0x78, 0x9c, 0x63, 0x62, 0x64, 0x02, 0, 0, 0x0d, 0, 6,
            ])
            .unwrap();
        assert_eq!(codec.next_packet().unwrap().unwrap(), [3, 1]);
        codec.compression = Some(1);
        assert_eq!(codec.next_packet().unwrap().unwrap(), [2, 1, 2]);
        assert!(codec.next_packet().unwrap().is_none());
    }
    #[test]
    fn compression_roundtrip_large_packet() {
        let packet: Vec<u8> = (0..200000).map(|i| (i % 251) as u8).collect();
        for compression in [None, Some(0), Some(256), Some(300000)] {
            let mut codec = Codec {
                compression,
                ..Codec::default()
            };
            let wire = codec.encode(&packet).unwrap();
            for byte in &wire[..wire.len() - 1] {
                codec.push(&[*byte]).unwrap();
                assert!(codec.next_packet().unwrap().is_none());
            }
            codec.push(&wire[wire.len() - 1..]).unwrap();
            assert_eq!(codec.next_packet().unwrap().unwrap(), packet);
        }
    }
    #[test]
    fn rejects_hostile_lengths_and_compression() {
        assert!(read_varint(&[255; 5]).is_err());
        for bytes in [
            vec![0],
            vec![255, 255, 255, 255, 15],
            vec![128, 128, 128, 8],
        ] {
            let mut codec = Codec::default();
            codec.push(&bytes).unwrap();
            assert!(codec.next_packet().is_err());
        }
        let mut codec = Codec {
            compression: Some(256),
            ..Codec::default()
        };
        // Claimed decompressed size one (below compression threshold).
        codec.push(&[2, 1, 0]).unwrap();
        assert!(codec.next_packet().is_err());
    }
}
