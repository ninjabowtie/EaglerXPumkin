//! Shared nonblocking runtime. No guest threads or socket cloning are required.
use crate::{config::Config, session::Session};
use eagler_ws::{self as ws, Decoder, OP_BIN, OP_CLOSE, OP_PING, OP_PONG, OP_TEXT};
use std::{
    io::{self, ErrorKind, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    time::{Duration, Instant},
};
use tracing::{info, warn};
const MAX_QUEUE: usize = 16 * 1024 * 1024;
const IO_BUDGET: usize = 256 * 1024;

#[derive(Default)]
struct Queue {
    bytes: Vec<u8>,
    offset: usize,
}
impl Queue {
    fn len(&self) -> usize {
        self.bytes.len() - self.offset
    }
    fn push(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.len() + bytes.len() > MAX_QUEUE {
            return Err(io::Error::other("send queue limit exceeded"));
        }
        if self.offset > 0 {
            self.bytes.drain(..self.offset);
            self.offset = 0;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn flush(&mut self, socket: &mut impl Write) -> io::Result<bool> {
        let mut sent = 0;
        while self.len() > 0 && sent < IO_BUDGET {
            let end = self.bytes.len().min(self.offset + IO_BUDGET - sent);
            match socket.write(&self.bytes[self.offset..end]) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.offset += n;
                    sent += n;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        if self.len() == 0 {
            self.bytes.clear();
            self.offset = 0;
        }
        Ok(sent > 0)
    }
}
#[derive(PartialEq, Eq)]
enum Mode {
    Sniff,
    Http,
    WebSocket,
    Raw,
}
struct Connection {
    client: TcpStream,
    backend: Option<TcpStream>,
    mode: Mode,
    header: Vec<u8>,
    decoder: Decoder,
    session: Session,
    client_out: Queue,
    backend_out: Queue,
    tunnel: bool,
    remote_paused: bool,
    started: Instant,
    active: Instant,
    closing: Option<Instant>,
    client_eof: bool,
    backend_eof: bool,
    client_write_closed: bool,
    backend_write_closed: bool,
}
impl Connection {
    fn new(client: TcpStream) -> io::Result<Self> {
        client.set_nonblocking(true)?;
        client.set_nodelay(true)?;
        Ok(Self {
            client,
            backend: None,
            mode: Mode::Sniff,
            header: Vec::new(),
            decoder: Decoder::default(),
            session: Session::default(),
            tunnel: false,
            remote_paused: false,
            client_out: Queue::default(),
            backend_out: Queue::default(),
            started: Instant::now(),
            active: Instant::now(),
            closing: None,
            client_eof: false,
            backend_eof: false,
            client_write_closed: false,
            backend_write_closed: false,
        })
    }
    fn connect(&mut self, cfg: &Config) -> io::Result<()> {
        // Numeric loopback only. Bound failed local connects to avoid an indefinite tick stall.
        let backend = TcpStream::connect_timeout(&cfg.backend_addr(), Duration::from_millis(20))?;
        backend.set_nonblocking(true)?;
        backend.set_nodelay(true)?;
        self.backend = Some(backend);
        Ok(())
    }
    fn close_ws(&mut self, code: u16) -> io::Result<()> {
        if self.closing.is_none() {
            self.client_out
                .push(&ws::encode_message(OP_CLOSE, &code.to_be_bytes()))?;
            self.closing = Some(Instant::now());
        }
        Ok(())
    }
    fn drain_session(&mut self, cfg: &Config) -> io::Result<()> {
        if self.session.connect_requested {
            self.session.connect_requested = false;
            if let Err(e) = self.connect(cfg) {
                warn!("backend connect failed: {e}");
                self.session.backend_output.clear();
                self.session
                    .fail("Could not connect to Pumpkin on the configured backend port");
            }
        }
        for message in std::mem::take(&mut self.session.client_output) {
            self.client_out
                .push(&ws::encode_message(message.opcode, &message.payload))?;
        }
        self.backend_out
            .push(&std::mem::take(&mut self.session.backend_output))?;
        if self.session.closing {
            self.close_ws(1000)?;
        }
        Ok(())
    }
    fn client_bytes(&mut self, bytes: &[u8], cfg: &Config, online: usize) -> io::Result<()> {
        if self.mode == Mode::Raw {
            return self.backend_out.push(bytes);
        }
        if matches!(self.mode, Mode::Sniff | Mode::Http) {
            self.header.extend_from_slice(bytes);
            if self.mode == Mode::Sniff {
                let check = self.header.len().min(4);
                if self.header[..check] != b"GET "[..check] {
                    self.mode = Mode::Raw;
                    self.connect(cfg)?;
                    return self.backend_out.push(&std::mem::take(&mut self.header));
                }
                if self.header.len() < 4 {
                    return Ok(());
                }
                self.mode = Mode::Http;
            }
            let end = self
                .header
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| p + 4);
            if let Some(end) = end {
                match ws::upgrade_response(&self.header[..end]) {
                    Ok(response) => self.client_out.push(&response)?,
                    Err(_) => {
                        self.client_out.push(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")?;
                        self.closing = Some(Instant::now());
                        return Ok(());
                    }
                }
                let request_line = std::str::from_utf8(&self.header[..end])
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("");
                let path = request_line.split_whitespace().nth(1).unwrap_or("/");
                self.tunnel = path.split('?').next() == Some("/tunnel");
                self.mode = Mode::WebSocket;
                self.decoder.push(&self.header[end..]).map_err(ws_error)?;
                self.header.clear();
            } else {
                if self.header.len() > 8192 {
                    return Err(io::Error::other("HTTP header too large"));
                }
                return Ok(());
            }
        } else {
            self.decoder.push(bytes).map_err(ws_error)?;
        }
        while let Some(message) = self.decoder.next_message().map_err(ws_error)? {
            match message.opcode {
                OP_PING => self
                    .client_out
                    .push(&ws::encode_message(OP_PONG, &message.payload))?,
                OP_PONG => {}
                OP_CLOSE => {
                    self.client_out
                        .push(&ws::encode_message(OP_CLOSE, &message.payload))?;
                    self.closing = Some(Instant::now());
                    break;
                }
                _ if self.tunnel => self.tunnel_message(message, cfg)?,
                _ => {
                    if let Err(e) = self.session.client_message(message, cfg, online) {
                        self.session.fail(&e);
                    }
                    self.drain_session(cfg)?;
                    if self.closing.is_some() {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
    fn tunnel_message(&mut self, message: ws::Message, cfg: &Config) -> io::Result<()> {
        if message.opcode == OP_BIN {
            if self.backend.is_none() {
                return Err(io::Error::other("tunnel is not connected"));
            }
            return self.backend_out.push(&message.payload);
        }
        if message.opcode != OP_TEXT || message.payload.len() > 4096 {
            return Err(io::Error::other("invalid tunnel control message"));
        }
        let control: serde_json::Value =
            serde_json::from_slice(&message.payload).map_err(io::Error::other)?;
        match control.get("type").and_then(|v| v.as_str()) {
            Some("connect") if self.backend.is_none() => {
                let host = control
                    .get("host")
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.is_empty() && v.len() <= 255)
                    .ok_or_else(|| io::Error::other("invalid tunnel host"))?;
                let port = control.get("port").and_then(|v| v.as_u64());
                // This is a gateway for this server, never an arbitrary TCP proxy.
                // The host is client metadata; the only destination is cfg.backend_addr().
                if port != Some(cfg.backend_port as u64) {
                    return Err(io::Error::other(
                        "tunnel target port does not match backend_port",
                    ));
                }
                match self.connect(cfg) {
                    Ok(()) => {
                        let response = serde_json::json!({"type": "connected", "host": host, "port": cfg.backend_port});
                        self.client_out.push(&ws::encode_message(
                            OP_TEXT,
                            response.to_string().as_bytes(),
                        ))?;
                    }
                    Err(e) => {
                        warn!("tunnel backend connect failed: {e}");
                        let response = serde_json::json!({"type": "error", "message": "Could not connect to Pumpkin"});
                        self.client_out.push(&ws::encode_message(
                            OP_TEXT,
                            response.to_string().as_bytes(),
                        ))?;
                        self.close_ws(1011)?;
                    }
                }
            }
            Some("flow") if self.backend.is_some() => {
                self.remote_paused = control
                    .get("paused")
                    .and_then(|v| v.as_bool())
                    .ok_or_else(|| io::Error::other("invalid tunnel flow control"))?;
            }
            Some("close") => self.close_ws(1000)?,
            _ => return Err(io::Error::other("unexpected tunnel control message")),
        }
        Ok(())
    }
    fn step(&mut self, cfg: &Config, online: usize) -> io::Result<bool> {
        if self.client_out.flush(&mut self.client)? {
            self.active = Instant::now();
        }
        if let Some(backend) = &mut self.backend
            && self.backend_out.flush(backend)?
        {
            self.active = Instant::now();
        }
        if let Some(since) = self.closing {
            return Ok(self.client_out.len() > 0 && since.elapsed() < Duration::from_secs(5));
        }
        let mut buffer = [0u8; 16384];
        let mut read = 0;
        while !self.client_eof
            && read < IO_BUDGET
            && self.backend_out.len() < MAX_QUEUE / 2
            && self.client_out.len() < MAX_QUEUE / 2
        {
            match self.client.read(&mut buffer) {
                Ok(0) => {
                    self.client_eof = true;
                    break;
                }
                Ok(n) => {
                    self.active = Instant::now();
                    read += n;
                    self.client_bytes(&buffer[..n], cfg, online)?;
                    if self.closing.is_some() {
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        read = 0;
        while self.closing.is_none()
            && !self.backend_eof
            && !self.remote_paused
            && read < IO_BUDGET
            && self.client_out.len() < MAX_QUEUE / 2
        {
            let Some(backend) = &mut self.backend else {
                break;
            };
            match backend.read(&mut buffer) {
                Ok(0) => {
                    self.backend_eof = true;
                    break;
                }
                Ok(n) => {
                    self.active = Instant::now();
                    read += n;
                    if self.mode == Mode::Raw {
                        self.client_out.push(&buffer[..n])?;
                    } else if self.tunnel {
                        self.client_out
                            .push(&ws::encode_message(OP_BIN, &buffer[..n]))?;
                    } else {
                        if let Err(e) = self.session.backend_bytes(&buffer[..n]) {
                            self.session.fail(&e);
                        }
                        self.drain_session(cfg)?;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        if self.mode == Mode::Raw {
            if let Some(backend) = &mut self.backend {
                self.backend_out.flush(backend)?;
                if self.client_eof && self.backend_out.len() == 0 && !self.backend_write_closed {
                    backend.shutdown(Shutdown::Write)?;
                    self.backend_write_closed = true;
                }
            }
            self.client_out.flush(&mut self.client)?;
            if self.backend_eof && self.client_out.len() == 0 && !self.client_write_closed {
                self.client.shutdown(Shutdown::Write)?;
                self.client_write_closed = true;
            }
            if self.client_write_closed && self.backend_write_closed {
                return Ok(false);
            }
        } else if self.client_eof {
            return Ok(false);
        } else if self.backend_eof {
            self.close_ws(1000)?;
        }
        self.client_out.flush(&mut self.client)?;
        if let Some(backend) = &mut self.backend {
            self.backend_out.flush(backend)?;
        }
        if self.active.elapsed() > Duration::from_secs(300) {
            return Ok(false);
        }
        if self.mode != Mode::Raw
            && !(self.session.is_playing() || self.tunnel && self.backend.is_some())
            && self.started.elapsed() > Duration::from_secs(30)
        {
            return Ok(false);
        }
        Ok(true)
    }
}
fn ws_error(error: ws::WsError) -> io::Error {
    io::Error::other(format!("WebSocket: {error:?}"))
}

pub struct Runtime {
    listener: TcpListener,
    config: Config,
    connections: Vec<Connection>,
}
impl Runtime {
    pub fn bind(config: Config) -> io::Result<Self> {
        config.validate().map_err(io::Error::other)?;
        let listener = TcpListener::bind(config.listen_addr())?;
        listener.set_nonblocking(true)?;
        info!(
            "Eagler listener on {} -> {} (MC protocol {})",
            listener.local_addr()?,
            config.backend_addr(),
            config.minecraft_protocol
        );
        Ok(Self {
            listener,
            config,
            connections: Vec::new(),
        })
    }
    pub fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }
    /// One bounded scheduler invocation. All sockets retain partial I/O state.
    pub fn poll(&mut self) {
        match self.listener.accept() {
            Ok((client, _)) if self.connections.len() < self.config.max_connections => {
                match Connection::new(client) {
                    Ok(connection) => self.connections.push(connection),
                    Err(e) => warn!("connection setup failed: {e}"),
                }
            }
            Ok(_) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(e) => warn!("accept failed: {e}"),
        }
        let online = self
            .connections
            .iter()
            .filter(|c| c.session.is_playing())
            .count();
        self.connections
            .retain_mut(|connection| match connection.step(&self.config, online) {
                Ok(alive) => alive,
                Err(e) => {
                    warn!("connection closed: {e}");
                    false
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn send_queue_preserves_partial_writes_and_appended_bytes() {
        struct ShortWriter {
            output: Vec<u8>,
            budget: usize,
            interrupted: bool,
        }
        impl Write for ShortWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(ErrorKind::Interrupted.into());
                }
                if self.budget == 0 {
                    return Err(ErrorKind::WouldBlock.into());
                }
                let n = self.budget.min(bytes.len()).min(3);
                self.output.extend_from_slice(&bytes[..n]);
                self.budget -= n;
                Ok(n)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut queue = Queue::default();
        queue.push(b"first message").unwrap();
        let mut writer = ShortWriter {
            output: Vec::new(),
            budget: 5,
            interrupted: false,
        };
        assert!(queue.flush(&mut writer).unwrap());
        assert_eq!(queue.len(), 8);
        assert!(!queue.flush(&mut writer).unwrap());
        queue.push(b" followed by second").unwrap();
        writer.budget = 100;
        queue.flush(&mut writer).unwrap();
        assert_eq!(writer.output, b"first message followed by second");
        assert_eq!(queue.len(), 0);
    }
}
