//! Minimal server-side WebSocket (RFC 6455) implementation over any `std::io`
//! Read + Write stream. Hand-rolled SHA-1 and base64 keep this crate dependency
//! free so it compiles cleanly for WASM (wasm32-wasip2 / wasm32-wasip3).
//!
//! This is deliberately small and explicit: Eaglercraft clients only use
//! binary frames plus ping/pong/close, and the server never masks its frames.
//!
//! Phase 0 spike scope: handshake + full frame round-trip (echo).

use std::io::{Read, Write};

// ---------------------------------------------------------------------------
// SHA-1 (RFC 3174) - only needed for the WS key hash; not security-sensitive.
// ---------------------------------------------------------------------------

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];

    // Padded message: msg + 0x80 + zeros + 64-bit big-endian bit length.
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for chunk in msg.as_chunks::<64>().0 {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Base64 (standard alphabet, padded)
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Debug)]
pub enum HandshakeError {
    Io(std::io::Error),
    /// Not an HTTP request / missing required upgrade headers.
    NotWebSocket,
    /// Looks like HTTP but not a WebSocket upgrade (e.g. a plain page request).
    PlainHttp,
}

impl From<std::io::Error> for HandshakeError {
    fn from(e: std::io::Error) -> Self {
        HandshakeError::Io(e)
    }
}

/// Performs the server side of the WS opening handshake by reading the HTTP
/// upgrade request from `inner` and writing the `101` response.
pub fn handshake(inner: &mut (impl Read + Write)) -> Result<(), HandshakeError> {
    // Read until "\r\n\r\n" (cap at 8 KiB).
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        let n = inner.read(&mut byte)?;
        if n == 0 {
            return Err(HandshakeError::NotWebSocket); // closed mid-request
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 8 * 1024 {
            return Err(HandshakeError::NotWebSocket);
        }
    }

    inner.write_all(&upgrade_response(&buf)?)?;
    inner.flush()?;
    Ok(())
}

/// Validate a complete HTTP request and build the upgrade response. The caller
/// owns incremental reads and must preserve bytes following the header terminator.
pub fn upgrade_response(request: &[u8]) -> Result<Vec<u8>, HandshakeError> {
    if request.len() > 8192 || !request.ends_with(b"\r\n\r\n") {
        return Err(HandshakeError::NotWebSocket);
    }
    let request = std::str::from_utf8(request).map_err(|_| HandshakeError::NotWebSocket)?;
    let mut lines = request.split("\r\n");
    let words: Vec<_> = lines.next().unwrap_or("").split_whitespace().collect();
    if words.len() != 3 || words[0] != "GET" || words[2] != "HTTP/1.1" {
        return Err(HandshakeError::PlainHttp);
    }
    let mut key = None;
    let (mut upgrade, mut connection, mut version, mut host) = (false, false, false, false);
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(HandshakeError::NotWebSocket)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("sec-websocket-key") {
            if key.replace(value).is_some() {
                return Err(HandshakeError::NotWebSocket);
            }
        } else if name.eq_ignore_ascii_case("upgrade") {
            upgrade |= value.eq_ignore_ascii_case("websocket");
        } else if name.eq_ignore_ascii_case("connection") {
            connection |= value
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("upgrade"));
        } else if name.eq_ignore_ascii_case("sec-websocket-version") {
            version = value == "13";
        } else if name.eq_ignore_ascii_case("host") {
            host = !value.is_empty();
        }
    }
    let key = key.ok_or(HandshakeError::NotWebSocket)?;
    // A canonical base64 encoding of a 16-byte nonce has 22 digits plus ==.
    if !upgrade
        || !connection
        || !version
        || !host
        || key.len() != 24
        || !key.ends_with("==")
        || !key.as_bytes()[..22].iter().all(|b| B64.contains(b))
        || !b"AQgw".contains(&key.as_bytes()[21])
    {
        return Err(HandshakeError::NotWebSocket);
    }
    let accept = base64_encode(&sha1(format!("{key}{WS_GUID}").as_bytes()));
    Ok(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").into_bytes())
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

pub const OP_CONT: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BIN: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

#[derive(Debug)]
pub enum WsError {
    Io(std::io::Error),
    Closed,
    Protocol(&'static str),
}

impl From<std::io::Error> for WsError {
    fn from(e: std::io::Error) -> Self {
        WsError::Io(e)
    }
}

/// A decoded data message (opcode + payload). Control frames are answered
/// internally by [`WsStream::read_message`].
#[derive(Debug, Clone)]
pub struct Message {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// Incremental server-side decoder. Partial frames and fragments survive between
/// scheduler calls; control frames are surfaced even inside fragmented messages.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    fragmented: Option<Message>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), WsError> {
        if self.buf.len() + bytes.len() > MAX_MESSAGE_SIZE + 14 {
            return Err(WsError::Protocol("receive buffer too large"));
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    pub fn next_message(&mut self) -> Result<Option<Message>, WsError> {
        loop {
            let Some((opcode, fin, payload, consumed)) = try_parse_frame(&self.buf)? else {
                return Ok(None);
            };
            self.buf.drain(..consumed);
            if opcode >= OP_CLOSE {
                if opcode == OP_CLOSE && payload.len() >= 2 {
                    let code = u16::from_be_bytes([payload[0], payload[1]]);
                    if !(matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999))
                        || std::str::from_utf8(&payload[2..]).is_err()
                    {
                        return Err(WsError::Protocol("invalid close status or reason"));
                    }
                }
                return Ok(Some(Message { opcode, payload }));
            }
            let message = if opcode == OP_CONT {
                let message = self
                    .fragmented
                    .as_mut()
                    .ok_or(WsError::Protocol("unexpected continuation"))?;
                if message.payload.len() + payload.len() > MAX_MESSAGE_SIZE {
                    return Err(WsError::Protocol("message too large"));
                }
                message.payload.extend_from_slice(&payload);
                if !fin {
                    continue;
                }
                self.fragmented.take().unwrap()
            } else {
                if self.fragmented.is_some() {
                    return Err(WsError::Protocol("unexpected data frame mid-message"));
                }
                let message = Message { opcode, payload };
                if !fin {
                    self.fragmented = Some(message);
                    continue;
                }
                message
            };
            if message.opcode == OP_TEXT && std::str::from_utf8(&message.payload).is_err() {
                return Err(WsError::Protocol("invalid text UTF-8"));
            }
            return Ok(Some(message));
        }
    }
}

/// Encode one complete, unmasked server frame.
pub fn encode_message(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x80 | opcode);
    if payload.len() < 126 {
        frame.push(payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    frame
}

/// Blocking convenience wrapper. Nonblocking runtimes should use [`Decoder`]
/// and retain their output queue across partial writes.
pub struct WsStream<T: Read + Write> {
    inner: T,
    decoder: Decoder,
}

impl<T: Read + Write> WsStream<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            decoder: Decoder::default(),
        }
    }
    pub fn get_ref(&self) -> &T {
        &self.inner
    }
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
    pub fn read_message(&mut self) -> Result<Message, WsError> {
        loop {
            if let Some(message) = self.decoder.next_message()? {
                match message.opcode {
                    OP_PING => self.write_message(OP_PONG, &message.payload)?,
                    OP_PONG => {}
                    OP_CLOSE => {
                        let _ = self.write_message(OP_CLOSE, &message.payload);
                        return Err(WsError::Closed);
                    }
                    _ => return Ok(message),
                }
                continue;
            }
            let mut bytes = [0; 4096];
            let n = match self.inner.read(&mut bytes) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if n == 0 {
                return Err(WsError::Closed);
            }
            self.decoder.push(&bytes[..n])?;
        }
    }
    pub fn write_message(&mut self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        self.inner.write_all(&encode_message(opcode, payload))?;
        self.inner.flush()?;
        Ok(())
    }
    pub fn close(&mut self) {
        let _ = self.write_message(OP_CLOSE, &[]);
    }
}

/// Attempts to parse one frame from `buf`. Returns
/// `(opcode, fin, payload, bytes_consumed)` or `None` if more bytes are needed.
#[allow(clippy::type_complexity)]
fn try_parse_frame(buf: &[u8]) -> Result<Option<(u8, bool, Vec<u8>, usize)>, WsError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    let opcode = b0 & 0x0F;
    let fin = (b0 & 0x80) != 0;
    let masked = (b1 & 0x80) != 0;
    if b0 & 0x70 != 0 {
        return Err(WsError::Protocol("reserved bits set"));
    }
    if !matches!(
        opcode,
        OP_CONT | OP_TEXT | OP_BIN | OP_CLOSE | OP_PING | OP_PONG
    ) {
        return Err(WsError::Protocol("unknown opcode"));
    }
    if !masked {
        return Err(WsError::Protocol("client frame must be masked"));
    }
    let len7 = (b1 & 0x7F) as usize;

    let mut header = 2usize;
    let payload_len = match len7 {
        126 => {
            if buf.len() < 4 {
                return Ok(None);
            }
            header = 4;
            let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
            if len < 126 {
                return Err(WsError::Protocol("non-minimal payload length"));
            }
            len
        }
        127 => {
            if buf.len() < 10 {
                return Ok(None);
            }
            header = 10;
            let mut b = [0u8; 8];
            b.copy_from_slice(&buf[2..10]);
            let len = u64::from_be_bytes(b);
            if len < 65536 || len & (1 << 63) != 0 {
                return Err(WsError::Protocol("invalid payload length"));
            }
            usize::try_from(len).map_err(|_| WsError::Protocol("frame too large"))?
        }
        n => n,
    };

    if opcode >= OP_CLOSE && (!fin || payload_len > 125) {
        return Err(WsError::Protocol("invalid control frame"));
    }
    if opcode == OP_CLOSE && payload_len == 1 {
        return Err(WsError::Protocol("invalid close payload"));
    }
    if payload_len > MAX_MESSAGE_SIZE {
        return Err(WsError::Protocol("frame too large"));
    }

    let mask_len = if masked { 4 } else { 0 };
    if buf.len() < header + mask_len {
        return Ok(None);
    }
    let mask = if masked {
        Some([
            buf[header],
            buf[header + 1],
            buf[header + 2],
            buf[header + 3],
        ])
    } else {
        None
    };
    let total = header + mask_len + payload_len;
    if buf.len() < total {
        return Ok(None);
    }
    let mut payload = buf[header + mask_len..total].to_vec();
    if let Some(mask) = mask {
        for (i, p) in payload.iter_mut().enumerate() {
            *p ^= mask[i % 4];
        }
    }
    Ok(Some((opcode, fin, payload, total)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Duplex {
        input: std::io::Cursor<Vec<u8>>,
        output: Vec<u8>,
        chunk_size: usize,
    }

    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let len = buf.len().min(self.chunk_size);
            self.input.read(&mut buf[..len])
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn client_frame(opcode: u8, fin: bool, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 126);
        let mask = [1, 2, 3, 4];
        let mut frame = vec![
            opcode | if fin { 0x80 } else { 0 },
            0x80 | payload.len() as u8,
        ];
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        frame
    }

    #[test]
    fn fragmented_messages_survive_partial_reads_and_interleaved_ping() {
        let mut input = client_frame(OP_BIN, false, b"hello ");
        input.extend(client_frame(OP_PING, true, b"ping"));
        input.extend(client_frame(OP_CONT, true, b"world"));
        input.extend(client_frame(OP_BIN, true, b"next"));
        for chunk_size in [1, 3, 4096] {
            let mut ws = WsStream::new(Duplex {
                input: std::io::Cursor::new(input.clone()),
                output: Vec::new(),
                chunk_size,
            });
            let message = ws.read_message().unwrap();
            assert_eq!(message.opcode, OP_BIN);
            assert_eq!(message.payload, b"hello world");
            assert_eq!(ws.get_ref().output, b"\x8a\x04ping");
            assert_eq!(ws.read_message().unwrap().payload, b"next");
        }
    }

    #[test]
    fn rejects_invalid_client_frames() {
        for frame in [
            vec![0x82, 0],                               // unmasked
            vec![0xc2, 0x80],                            // reserved bits
            vec![0x83, 0x80],                            // reserved opcode
            vec![0x09, 0x80],                            // fragmented ping
            vec![0x89, 0xfe, 0, 126],                    // oversized ping
            vec![0x82, 0xfe, 0, 1],                      // non-minimal length
            vec![0x88, 0x81],                            // one-byte close status
            vec![0x82, 0xff, 0x80, 0, 0, 0, 0, 0, 0, 0], // invalid 64-bit length
        ] {
            assert!(matches!(try_parse_frame(&frame), Err(WsError::Protocol(_))));
        }
        let mut ws = WsStream::new(Duplex {
            input: std::io::Cursor::new(client_frame(OP_CONT, true, b"orphan")),
            output: Vec::new(),
            chunk_size: 4096,
        });
        assert!(matches!(ws.read_message(), Err(WsError::Protocol(_))));
    }

    #[test]
    fn incremental_decoder_retains_fragments_between_polls() {
        let mut decoder = Decoder::default();
        decoder
            .push(&client_frame(OP_BIN, false, b"first"))
            .unwrap();
        assert!(decoder.next_message().unwrap().is_none());
        for byte in client_frame(OP_CONT, true, b"second") {
            decoder.push(&[byte]).unwrap();
        }
        assert_eq!(
            decoder.next_message().unwrap().unwrap().payload,
            b"firstsecond"
        );
    }

    #[test]
    fn upgrade_matches_rfc_example_and_rejects_missing_headers() {
        let request = b"GET /tunnel HTTP/1.1\r\nHost: localhost\r\nUpgrade: WebSocket\r\nConnection: keep-alive, Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        let response = upgrade_response(request).unwrap();
        assert!(
            String::from_utf8(response)
                .unwrap()
                .contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
        );
        let text = std::str::from_utf8(request).unwrap();
        for line in [
            "Upgrade: WebSocket\r\n",
            "Connection: keep-alive, Upgrade\r\n",
            "Sec-WebSocket-Version: 13\r\n",
            "Host: localhost\r\n",
        ] {
            assert!(upgrade_response(text.replace(line, "").as_bytes()).is_err());
        }
        assert!(
            upgrade_response(
                text.replace("Sec-WebSocket-Version: 13", "Sec-WebSocket-Version: 12")
                    .as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn extended_lengths_and_large_binary_messages() {
        for len in [126, 65535, 65536, 200000] {
            let payload: Vec<_> = (0..len).map(|i| (i % 251) as u8).collect();
            let encoded = encode_message(OP_BIN, &payload);
            assert_eq!(encoded[1], if len <= 65535 { 126 } else { 127 });
            let header_len = if len <= 65535 { 4 } else { 10 };
            let mut wire = encoded[..header_len].to_vec();
            wire[1] |= 128;
            wire.extend_from_slice(&[1, 2, 3, 4]);
            wire.extend(
                payload
                    .iter()
                    .enumerate()
                    .map(|(i, b)| b ^ [1, 2, 3, 4][i % 4]),
            );
            let mut decoder = Decoder::default();
            for chunk in wire.chunks(97) {
                decoder.push(chunk).unwrap();
            }
            assert_eq!(decoder.next_message().unwrap().unwrap().payload, payload);
        }
    }

    #[test]
    fn sha1_rfc_vectors() {
        let h = sha1(b"abc");
        assert_eq!(base64_encode(&h), "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=");
        // Vector: "abc" -> a9993e364706816aba3e25717850c26c9cd0d89d
        assert_eq!(
            h.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn b64_rfc4648() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn frame_roundtrip() {
        // Build a masked client frame, parse it back.
        let payload = b"hello eagler";
        let mut masked = payload.to_vec();
        let mask = [0x37, 0xFA, 0x21, 0x3D];
        for (i, p) in masked.iter_mut().enumerate() {
            *p ^= mask[i % 4];
        }
        let mut buf = vec![0x82, 0x80 | payload.len() as u8];
        buf.extend_from_slice(&mask);
        buf.extend_from_slice(&masked);

        let (op, fin, pl, consumed) = try_parse_frame(&buf).unwrap().unwrap();
        assert_eq!(op, OP_BIN);
        assert!(fin);
        assert_eq!(pl, payload);
        assert_eq!(consumed, buf.len());
    }

    /// Encode/decode micro-benchmark for representative Minecraft packet sizes.
    /// Each round decodes a masked client frame and encodes the server reply.
    /// This measures codec work only, without sockets or a Minecraft backend.
    #[test]
    fn frame_codec_micro_benchmark() {
        fn masked_frame(opcode: u8, fin: bool, payload: &[u8]) -> Vec<u8> {
            let mask = [0x37, 0xFA, 0x21, 0x3D];
            let mut frame = vec![opcode | if fin { 0x80 } else { 0 }, 0x80];
            let len = payload.len();
            if len < 126 {
                frame[1] = 0x80 | len as u8;
            } else if len <= u16::MAX as usize {
                frame[1] = 0x80 | 126;
                frame.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                frame[1] = 0x80 | 127;
                frame.extend_from_slice(&(len as u64).to_be_bytes());
            }
            frame.extend_from_slice(&mask);
            frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
            frame
        }
        for len in [64usize, 1024, 65_536, 262_144] {
            let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let frame = masked_frame(OP_BIN, true, &payload);
            let expected_reply = encode_message(OP_BIN, &payload);
            let rounds = 128usize;
            let mut decoder = Decoder::default();
            let start = std::time::Instant::now();
            let mut decoded_bytes = 0usize;
            for _ in 0..rounds {
                decoder.push(&frame).unwrap();
                match decoder.next_message().unwrap() {
                    Some(message) => {
                        assert_eq!(message.payload, payload);
                        let reply = encode_message(message.opcode, &message.payload);
                        assert_eq!(reply, expected_reply);
                        decoded_bytes += message.payload.len();
                        std::hint::black_box(reply);
                    }
                    None => panic!("benchmark frame was not decoded"),
                }
            }
            let elapsed = start.elapsed().as_secs_f64();
            let bytes = len as f64 * rounds as f64;
            let throughput_kibps = bytes / elapsed / 1024.0;
            let mean_us = elapsed * 1_000_000.0 / rounds as f64;
            eprintln!(
                "frame codec micro-benchmark: payload {len} bytes, {rounds} decode+encode rounds -> {throughput_kibps:.1} payload KiB/s, {mean_us:.1} us/round"
            );
            assert_eq!(decoded_bytes, len * rounds);
        }
    }
}
