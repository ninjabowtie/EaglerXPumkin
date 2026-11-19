//! Minecraft TCP framing and compression. Eagler messages carry packet bodies,
//! whereas the backend expects VarInt lengths and negotiated zlib compression.
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use std::io::Write;

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
    offset: usize,
    pub compression: Option<usize>,
}
impl Codec {
    pub fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if self.input.len() - self.offset + bytes.len() > MAX_PACKET + 65536 {
            return Err("backend receive buffer too large".into());
        }
        // Compact at most once per incoming read, never once per packet.
        if self.offset > 0 {
            self.input.copy_within(self.offset.., 0);
            self.input.truncate(self.input.len() - self.offset);
            self.offset = 0;
        }
        self.input.extend_from_slice(bytes);
        Ok(())
    }
    pub fn next_packet(&mut self) -> Result<Option<Vec<u8>>> {
        let Some((len, header)) = read_varint(&self.input[self.offset..])? else {
            return Ok(None);
        };
        if len <= 0 || len as usize > MAX_PACKET {
            return Err("invalid backend packet length".into());
        }
        let start = self.offset + header;
        let end = start + len as usize;
        if self.input.len() < end {
            return Ok(None);
        }
        let packet = decode_body(&self.input[start..end], self.compression)?;
        self.offset = end;
        if self.offset == self.input.len() {
            self.input.clear();
            self.offset = 0;
        }
        Ok(Some(packet))
    }
    pub fn encode(&self, packet: &[u8]) -> Result<Vec<u8>> {
        if packet.is_empty() || packet.len() > MAX_PACKET {
            return Err("invalid Minecraft packet size".into());
        }
        let Some(threshold) = self.compression else {
            let mut framed = Vec::with_capacity(packet.len() + 5);
            varint(&mut framed, packet.len() as i32);
            framed.extend_from_slice(packet);
            return Ok(framed);
        };
        let mut data = Vec::new();
        if packet.len() >= threshold {
            varint(&mut data, packet.len() as i32);
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(packet).map_err(|e| e.to_string())?;
            data.extend(encoder.finish().map_err(|e| e.to_string())?);
        } else {
            data.push(0);
            data.extend_from_slice(packet);
        }
        if data.len() > MAX_PACKET {
            return Err("encoded Minecraft packet exceeds wire size limit".into());
        }
        let mut framed = Vec::with_capacity(data.len() + 5);
        varint(&mut framed, data.len() as i32);
        framed.extend(data);
        Ok(framed)
    }
}

/// Decode directly from receive storage without copying the compressed body.
/// Explicit stream completion preserves checksum/trailer validation, with
/// output bounded by the declared length and one extra byte to detect overflow.
fn decode_body(data: &[u8], compression: Option<usize>) -> Result<Vec<u8>> {
    let Some(threshold) = compression else {
        return Ok(data.to_vec());
    };
    let (size, offset) = read_varint(data)?.ok_or("missing uncompressed length")?;
    if size == 0 {
        if data.len() == offset || data.len() - offset >= threshold {
            return Err("empty or above-threshold uncompressed packet".into());
        }
        return Ok(data[offset..].to_vec());
    }
    if size < 0 || size as usize > MAX_PACKET || (size as usize) < threshold {
        return Err("invalid uncompressed length".into());
    }
    let mut decoder = Decompress::new(true);
    let mut out = Vec::with_capacity(size as usize + 1);
    let status = decoder
        .decompress_vec(&data[offset..], &mut out, FlushDecompress::Finish)
        .map_err(|e| e.to_string())?;
    if status != Status::StreamEnd
        || out.len() != size as usize
        || decoder.total_in() as usize != data.len() - offset
    {
        return Err("incomplete or mismatched compressed packet".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coalesced_packets_and_partial_tails_survive_compaction() {
        let packets: Vec<_> = (1..=256).map(|n| vec![n as u8; n]).collect();
        for compression in [None, Some(0), Some(128)] {
            let mut codec = Codec {
                compression,
                ..Default::default()
            };
            let wire: Vec<_> = packets
                .iter()
                .flat_map(|p| codec.encode(p).unwrap())
                .collect();
            for chunk_size in [1, 7, 4096] {
                let mut decoded = Vec::new();
                for chunk in wire.chunks(chunk_size) {
                    codec.push(chunk).unwrap();
                    while let Some(packet) = codec.next_packet().unwrap() {
                        decoded.push(packet);
                    }
                }
                assert_eq!(decoded, packets);
            }
        }
    }

    #[test]
    fn encoded_wire_size_is_bounded_and_trailing_compressed_data_is_rejected() {
        let codec = Codec {
            compression: Some(MAX_PACKET + 1),
            ..Default::default()
        };
        assert!(codec.encode(&vec![42; MAX_PACKET]).is_err());
        let codec = Codec {
            compression: Some(1),
            ..Default::default()
        };
        let wire = codec.encode(&[1, 2, 3]).unwrap();
        let (_, header) = read_varint(&wire).unwrap().unwrap();
        let mut frame = Vec::new();
        varint(&mut frame, (wire.len() - header + 1) as i32);
        frame.extend_from_slice(&wire[header..]);
        frame.push(0);
        let mut decoder = codec;
        decoder.push(&frame).unwrap();
        assert!(decoder.next_packet().is_err());
    }

    #[test]
    fn rejects_truncated_zlib_and_empty_compressed_packets() {
        let codec = Codec {
            compression: Some(1),
            ..Default::default()
        };
        let wire = codec.encode(&[1, 2, 3, 4]).unwrap();
        let (_, header) = read_varint(&wire).unwrap().unwrap();
        for missing in 1..=4 {
            let body = &wire[header..wire.len() - missing];
            let mut frame = Vec::new();
            varint(&mut frame, body.len() as i32);
            frame.extend_from_slice(body);
            let mut decoder = Codec {
                compression: Some(1),
                ..Default::default()
            };
            decoder.push(&frame).unwrap();
            assert!(
                decoder.next_packet().is_err(),
                "accepted zlib stream missing {missing} trailer bytes"
            );
        }
        let mut decoder = Codec {
            compression: Some(0),
            ..Default::default()
        };
        decoder.push(&[1, 0]).unwrap();
        assert!(decoder.next_packet().is_err(), "accepted empty packet body");
    }

    #[test]
    fn compression_threshold_zero_requires_compressed_packets() {
        let mut decoder = Codec {
            compression: Some(0),
            ..Default::default()
        };
        decoder.push(&[2, 0, 1]).unwrap();
        assert!(decoder.next_packet().is_err());
    }

    #[test]
    #[ignore = "manual coalesced-buffer performance measurement"]
    fn coalesced_batch_benchmark() {
        let encoder = Codec::default();
        let frame = encoder.encode(&[42; 64]).unwrap();
        let bytes = frame.repeat(10000);
        let start = std::time::Instant::now();
        for _ in 0..3 {
            let mut codec = Codec::default();
            codec.push(&bytes).unwrap();
            for _ in 0..10000 {
                assert_eq!(codec.next_packet().unwrap().unwrap(), [42; 64]);
            }
            assert!(codec.next_packet().unwrap().is_none());
        }
        eprintln!(
            "Minecraft 30000 coalesced 64-byte packets: {:?}",
            start.elapsed()
        );
    }

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
