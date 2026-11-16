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
// SHA-1 (RFC 3174) — only needed for the WS key hash; not security-sensitive.
// ---------------------------------------------------------------------------

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];

    // Padded message: msg + 0x80 + zeros + 64-bit big-endian bit length.
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for chunk in msg.chunks_exact(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
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
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
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
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
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

    let request = String::from_utf8_lossy(&buf).into_owned();
    let mut method_upgrade = false;
    let mut key: Option<&str> = None;
    for line in request.lines() {
        if line.starts_with("GET ") {
            method_upgrade = true;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("sec-websocket-key:") {
            key = request.lines().nth(0).and_then(|_| None).or(Some(
                // keep original casing: find the raw line
                line.split_once(':').map(|(_, v)| v.trim()).unwrap_or(""),
            ));
            let _ = v;
        }
        let _ = lower;
    }
    if !method_upgrade {
        return Err(HandshakeError::PlainHttp);
    }
    // Simpler robust parse: case-insensitive scan of the raw request.
    let mut raw_key: Option<String> = None;
    for line in request.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("sec-websocket-key") {
                raw_key = Some(value.trim().to_string());
            }
        }
    }
    let key = match raw_key.as_deref() {
        Some(k) if !k.is_empty() => k,
        _ => return Err(HandshakeError::NotWebSocket),
    };
    let _ = key.as_bytes();

    // Sec-WebSocket-Accept = base64(sha1(key + GUID))
    let mut accept_input = key.to_string();
    accept_input.push_str(WS_GUID);
    let accept = base64_encode(&sha1(accept_input.as_bytes()));

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    inner.write_all(response.as_bytes())?;
    inner.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

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

/// Buffered WS framing over a byte stream. The underlying stream is expected
/// to already have completed the HTTP upgrade (see [`handshake`]).
pub struct WsStream<T: Read + Write> {
    inner: T,
    buf: Vec<u8>, // leftover bytes from reads, for reassembly
}

impl<T: Read + Write> WsStream<T> {
    pub fn new(inner: T) -> Self {
        Self { inner, buf: Vec::new() }
    }

    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Reads exactly one full frame (including continuation), transparently
    /// answering ping and replying to close per RFC 6455. Returns `Err(WsError::Closed)`
    /// when the peer closes cleanly.
    pub fn read_message(&mut self) -> Result<Message, WsError> {
        loop {
            // Parse as much as we can from buf.
            if let Some((opcode, fin, payload, consumed)) = try_parse_frame(&self.buf)? {
                self.buf.drain(..consumed);

                match opcode {
                    OP_PING => {
                        self.write_message(OP_PONG, &payload)?;
                        continue;
                    }
                    OP_CLOSE => {
                        // Echo close then signal closed.
                        let _ = self.write_message(OP_CLOSE, &payload);
                        return Err(WsError::Closed);
                    }
                    OP_PONG => continue, // ignore unsolicited pongs
                    op if fin => return Ok(Message { opcode: op, payload }),
                    _ => {
                        // Start of a fragmented message: keep reading continuation.
                        let mut acc = payload;
                        let first_opcode = op;
                        loop {
                            let (cop, cfin, cpl, c) = try_parse_frame(&fill(&mut self.buf, &mut self.inner)?)?
                                .ok_or(WsError::Protocol("incomplete frame"))?;
                            self.buf.drain(..c);
                            match cop {
                                OP_CONT => {
                                    acc.extend_from_slice(&cpl);
                                    if cfin {
                                        return Ok(Message { opcode: first_opcode, payload: acc });
                                    }
                                }
                                OP_PING => self.write_message(OP_PONG, &cpl)?,
                                OP_CLOSE => return Err(WsError::Closed),
                                OP_PONG => {}
                                _ => return Err(WsError::Protocol("unexpected data frame mid-message")),
                            }
                        }
                    }
                }
            }
            // Need more bytes.
            self.buf = fill(&mut self.buf, &mut self.inner)?;
        }
    }

    /// Writes a complete frame with the given opcode. Server frames are unmasked.
    pub fn write_message(&mut self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        let mut frame = Vec::with_capacity(payload.len() + 10);
        frame.push(0x80 | opcode); // FIN + opcode
        let len = payload.len();
        if len < 126 {
            frame.push(len as u8);
        } else if len <= u16::MAX as usize {
            frame.push(126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        frame.extend_from_slice(payload);
        self.inner.write_all(&frame)?;
        self.inner.flush()?;
        Ok(())
    }

    /// Sends a close frame (best effort).
    pub fn close(&mut self) {
        let _ = self.write_message(OP_CLOSE, &[]);
    }
}

/// Ensures `buf` has at least a few more bytes, returning the extended buffer.
fn fill(buf: &mut Vec<u8>, inner: &mut impl Read) -> Result<Vec<u8>, WsError> {
    let mut tmp = [0u8; 4096];
    let n = inner.read(&mut tmp)?;
    if n == 0 {
        return Err(WsError::Closed);
    }
    buf.extend_from_slice(&tmp[..n]);
    Ok(std::mem::take(buf))
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
    let len7 = (b1 & 0x7F) as usize;

    let mut header = 2usize;
    let payload_len = match len7 {
        126 => {
            if buf.len() < 4 {
                return Ok(None);
            }
            header = 4;
            u16::from_be_bytes([buf[2], buf[3]]) as usize
        }
        127 => {
            if buf.len() < 10 {
                return Ok(None);
            }
            header = 10;
            let mut b = [0u8; 8];
            b.copy_from_slice(&buf[2..10]);
            u64::from_be_bytes(b) as usize
        }
        n => n,
    };

    if payload_len > 16 * 1024 * 1024 {
        return Err(WsError::Protocol("frame too large"));
    }

    let mask_len = if masked { 4 } else { 0 };
    if buf.len() < header + mask_len {
        return Ok(None);
    }
    let mask = if masked {
        Some([buf[header], buf[header + 1], buf[header + 2], buf[header + 3]])
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

    #[test]
    fn sha1_rfc_vectors() {
        let h = sha1(b"abc");
        assert_eq!(base64_encode(&h), "qU+Pq5CIqFu7Etheri6tLb8Q7KcQ=");
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
}
