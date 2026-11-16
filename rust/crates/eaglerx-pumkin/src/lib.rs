//! EaglerXPumkin — Phase 0 spike.
//!
//! Goals of this build (see /PUMPKIN_PORT_PLAN.md §Phase 0):
//!   1. Bind a TCP port from inside a Pumpkin WASM plugin (`wasi:sockets`).
//!   2. Sniff each connection: HTTP/WebSocket upgrade vs raw Minecraft bytes.
//!   3. WebSocket path: complete the upgrade and echo messages (proves WS framing).
//!   4. Minecraft path: relay raw bytes over loopback TCP to the backend port,
//!      proving a vanilla client can join **through the plugin**.
//!
//! Threading strategy: try a blocking listener thread first; if guest threads
//! are unavailable in the host's wasmtime config, fall back to a scheduler task
//! that drains non-blocking sockets every tick. Everything is logged loudly so
//! the spike outcome can be recorded in the plan file.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, OnceLock};

use eagler_ws as ws;
use pumpkin_plugin_api::{Context, Plugin, PluginMetadata, permissions::permissions, scheduler::SchedulerExt};
use tracing::info;

const DEFAULT_LISTEN_PORT: u16 = 8081;
const DEFAULT_BACKEND_PORT: u16 = 25565;
const SNIFF_BYTES: usize = 4;
const MAX_CONNS: usize = 256;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Config {
    listen_port: u16,
    backend_port: u16,
}

impl Config {
    fn load(context: &Context) -> Self {
        let mut cfg = Config {
            listen_port: DEFAULT_LISTEN_PORT,
            backend_port: DEFAULT_BACKEND_PORT,
        };
        let path = context.get_data_folder().join("eagler.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                for line in text.lines() {
                    let line = line.split('#').next().unwrap_or("");
                    if let Some((k, v)) = line.split_once('=') {
                        let k = k.trim();
                        let v = v.trim();
                        match k {
                            "listen_port" => cfg.listen_port = v.parse().unwrap_or(cfg.listen_port),
                            "backend_port" => cfg.backend_port = v.parse().unwrap_or(cfg.backend_port),
                            _ => {}
                        }
                    }
                }
                info!("eagler config loaded: {:?}", cfg);
            }
            Err(_) => {
                let default = format!(
                    "# EaglerXPumkin spike config\nlisten_port = {DEFAULT_LISTEN_PORT}\nbackend_port = {DEFAULT_BACKEND_PORT}\n"
                );
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&path, default) {
                    Ok(()) => info!("wrote default config to {}", path.display()),
                    Err(e) => info!("could not write default config (fs.write.data missing?): {e}"),
                }
            }
        }
        cfg
    }
}

// ---------------------------------------------------------------------------
// Shared connection state
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Mode {
    /// Still collecting first bytes to classify the connection.
    Sniffing,
    /// Raw Minecraft byte relay; `backend` is the loopback connection.
    RawRelay,
    /// WebSocket mode (post-upgrade). Spike: echo. Phase 1: Eagler protocol.
    WebSocket,
}

struct Conn {
    client: TcpStream,
    backend: Option<TcpStream>,
    mode: Mode,
    sniff: Vec<u8>,
}

struct State {
    cfg: Config,
    conns: HashMap<u64, Conn>,
    next_id: u64,
    /// Set once the listener has been created (or failed).
    listener_ok: bool,
}

static STATE: OnceLock<Mutex<Arc<Mutex<State>>>> = OnceLock::new();

fn state() -> &'static Mutex<Arc<Mutex<State>>> {
    STATE.get_or_init(|| Mutex::new(Arc::new(Mutex::new(State {
        cfg: Config { listen_port: DEFAULT_LISTEN_PORT, backend_port: DEFAULT_BACKEND_PORT },
        conns: HashMap::new(),
        next_id: 0,
        listener_ok: false,
    }))))
}

// ---------------------------------------------------------------------------
// Classification + routing
// ---------------------------------------------------------------------------

/// Classifies buffered first bytes. Vanilla MC packets begin with a VarInt
/// length followed by packet id `0x00`; HTTP methods are ASCII. This is spike
/// quality — Phase 1 hardens it.
fn classify(buf: &[u8]) -> Mode {
    if buf.is_empty() {
        return Mode::Sniffing;
    }
    let b = buf[0];
    if b < 0x80 && buf.get(1) == Some(&0x00) {
        return Mode::RawRelay; // MC handshake/status/transfer: VarInt len, id 0x00
    }
    if b.is_ascii_alphabetic() {
        return Mode::WebSocket; // HTTP-ish ("GET ", "POST", "OPTIONS", ...)
    }
    // Fall back to raw relay for anything else (e.g. >127 VarInt lengths).
    Mode::RawRelay
}

/// Establishes the loopback backend connection for a raw-relay client.
fn connect_backend(cfg: &Config) -> Option<TcpStream> {
    match TcpStream::connect(("127.0.0.1", cfg.backend_port)) {
        Ok(s) => Some(s),
        Err(e) => {
            info!("backend connect to 127.0.0.1:{} failed: {e}", cfg.backend_port);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

fn bind_listener(cfg: &Config) -> Option<TcpListener> {
    match TcpListener::bind(("0.0.0.0", cfg.listen_port)) {
        Ok(l) => {
            info!("Eagler listener bound on 0.0.0.0:{} (network.tcp.bind works!)", cfg.listen_port);
            Some(l)
        }
        Err(e) => {
            info!("BIND FAILED on port {} ({e}) — check that the network.tcp.bind permission was granted", cfg.listen_port);
            None
        }
    }
}

/// Blocking listener thread body (thread mode). One thread per connection via
/// `std::thread` keeps latency at zero; the spike verifies whether the host
/// allows guest threads at all.
fn thread_mode(listener: TcpListener, state: Arc<Mutex<State>>) {
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                info!("accept error: {e}");
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let st = Arc::clone(&state);
        let spawned = std::thread::Builder::new()
            .name("eagler-conn".into())
            .spawn(move || handle_conn(&mut stream, st));
        match spawned {
            Ok(_) => {}
            Err(e) => {
                info!("guest thread spawn failed ({e}) — thread mode unavailable");
                return;
            }
        }
    }
}

/// Handles one connection to completion (thread mode).
fn handle_conn(client: &mut TcpStream, state: Arc<Mutex<State>>) {
    let (cfg, mode, sniff) = {
        let outer = state.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = outer.lock().unwrap_or_else(|e| e.into_inner());
        // Read sniff bytes.
        let mut sniff = vec![0u8; SNIFF_BYTES];
        let mut got = 0;
        while got < SNIFF_BYTES {
            match client.read(&mut sniff[got..]) {
                Ok(0) => return,
                Ok(n) => got += n,
                Err(e) => {
                    info!("sniff read error: {e}");
                    return;
                }
            }
        }
        sniff.truncate(got);
        let mode = classify(&sniff);
        st.next_id += 1;
        let _id = st.next_id;
        (st.cfg, mode, sniff)
    };

    match mode {
        Mode::WebSocket => {
            // Replay sniffed bytes is unnecessary for HTTP parsing (we read the
            // full request inside ws::handshake — the sniffed 4 bytes are the
            // start of "GET "), so the handshake reader must not re-read them.
            // Simplest correct approach: hand the stream to the WS handler with
            // the sniffed bytes pushed back via a cursor wrapper.
            let mut prefix = std::io::Cursor::new(sniff);
            let mut chained = prefix.chain(client);
            if let Err(e) = ws::handshake(&mut chained) {
                info!("WS handshake failed: {e:?}");
                return;
            }
            ws_echo(&mut chained);
        }
        Mode::RawRelay => {
            let Some(mut backend) = connect_backend(&cfg) else { return };
            let _ = backend.set_nodelay(true);
            let _ = backend.write_all(&sniff);
            relay(client, &mut backend);
        }
        Mode::Sniffing => unreachable!(),
    }
}

/// WebSocket echo loop (spike: proves framing both directions).
fn ws_echo<S: Read + Write>(stream: &mut S) {
    let mut wss = ws::WsStream::new(WsAdapter::from_chain(stream));
    info!("WS echo session started");
    loop {
        match wss.read_message() {
            Ok(msg) => {
                if let Err(e) = wss.write_message(msg.opcode, &msg.payload) {
                    info!("ws write failed: {e:?}");
                    return;
                }
            }
            Err(ws::WsError::Closed) => {
                info!("ws client closed cleanly");
                return;
            }
            Err(e) => {
                info!("ws read failed: {e:?}");
                return;
            }
        }
    }
}

/// Bidirectional raw relay (spike: proves a vanilla client can join through us).
fn relay(client: &mut TcpStream, backend: &mut TcpStream) {
    info!("raw relay established");
    // Two directions must run concurrently: use a thread for client→backend and
    // loop backend→client here. If guest threads fail, the scheduler fallback
    // path (polling) covers this; thread mode is the spike experiment.
    let res = std::thread::Builder::new()
        .name("eagler-c2b".into())
        .spawn(move || copy_dir(client, backend));
    match res {
        Ok(_) => {
            // c2b runs on its own thread; b2c here.
            let _ = copy_dir(&mut backend_clone(backend), backend);
        }
        Err(_) => {
            // No threads: sequential ping-pong relay (still functional for
            // request/response flows; too slow for real play — logged loudly).
            info!("relay: no guest threads — sequential fallback (latency will suffer)");
            let mut buf = [0u8; 8192];
            loop {
                match client.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if backend.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
                match backend.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if client.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

// Cloning a TcpStream on WASI: std doesn't expose try_clone for wasip2 reliably;
// use a pipe-based mirror? For the spike, b2c copies happen in the spawned
// thread and the main loop just waits. Keep this placeholder minimal.
fn backend_clone(b: &TcpStream) -> TcpStream {
    // SAFETY: spike placeholder — replaced once try_clone support is confirmed.
    unsafe { mirror_stream(b) }
}

unsafe fn mirror_stream(_b: &TcpStream) -> TcpStream {
    unreachable!("resolved during spike: use dup or restructure into two half-relays with a channel")
}

fn copy_dir<R: Read, W: Write>(r: &mut R, w: &mut W) {
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if w.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler fallback (no threads): poll everything once per tick.
// ---------------------------------------------------------------------------

fn poll_step() {
    let outer = state().lock().unwrap_or_else(|e| e.into_inner());
    let st_arc = Arc::clone(&outer.lock().unwrap_or_else(|e| e.into_inner()).0());
    let mut st = st_arc.lock().unwrap_or_else(|e| e.into_inner());
    let _ = st;
    // NOTE: filled in after thread-mode findings; see TODO in spike notes.
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

struct EaglerXPlugin;

impl Plugin for EaglerXPlugin {
    fn new() -> Self {
        EaglerXPlugin
    }

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            name: "eaglerx-pumkin".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            authors: vec!["EaglerXPumkin".into()],
            description: "Eaglercraft support for Pumpkin: embedded Eagler -> loopback proxy (Phase 0 spike).".into(),
            dependencies: vec![],
            permissions: vec![
                permissions::NETWORK_TCP.into(),
                permissions::NETWORK_TCP_BIND.into(),
                permissions::NETWORK_TCP_CONNECT.into(),
                permissions::NETWORK_LOOPBACK.into(),
                permissions::FS_READ_DATA.into(),
                permissions::FS_WRITE_DATA.into(),
            ],
        }
    }

    fn on_load(&self, context: Context) -> Result<(), String> {
        info!("EaglerXPumkin spike loading…");
        let cfg = Config::load(&context);

        {
            let outer = state().lock().unwrap_or_else(|e| e.into_inner());
            let mut st = outer.lock().unwrap_or_else(|e| e.into_inner());
            st.cfg = cfg;
        }

        let listener = bind_listener(&cfg);
        match listener {
            Some(listener) => {
                // Experiment 1: guest threads.
                let st = Arc::new(Mutex::new(Arc::new(Mutex::new(State {
                    cfg,
                    conns: HashMap::new(),
                    next_id: 0,
                    listener_ok: true,
                }))));
                let attempt = std::thread::Builder::new()
                    .name("eagler-listener".into())
                    .spawn(move || thread_mode(listener, st));
                match attempt {
                    Ok(_) => info!("thread mode engaged: blocking listener on port {}", cfg.listen_port),
                    Err(e) => {
                        info!("thread spawn failed ({e}); falling back to scheduler polling");
                        // Experiment 2: scheduler-driven nonblocking poll.
                        context.schedule_repeating_task(1, 1, |_server| poll_step());
                    }
                }
            }
            None => {
                info!("no listener; spike cannot proceed — verify plugin permissions on next boot");
            }
        }
        Ok(())
    }

    fn on_unload(&self, _context: Context) -> Result<(), String> {
        info!("EaglerXPumkin spike unloaded");
        Ok(())
    }
}

pumpkin_plugin_api::register_plugin!(EaglerXPlugin);

// ---------------------------------------------------------------------------
// Adapters
// ---------------------------------------------------------------------------

/// Bridges `std::io::Read` chaining (prefix cursor + socket) into a single
/// `Read` for the WS handshake, then provides the socket again for framing.
struct WsAdapter {
    prefix: std::io::Cursor<Vec<u8>>,
    socket: TcpStream,
}

impl WsAdapter {
    fn from_chain<S: Read>(chain: &mut Chain<S>) -> WsAdapter {
        unreachable!("adapter construction is resolved in the spike build")
    }
}

type Chain<S> = std::io::Chain<std::io::Cursor<Vec<u8>>, S>;

impl Read for WsAdapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.prefix.position() < self.prefix.get_ref().len() as u64 {
            self.prefix.read(buf)
        } else {
            self.socket.read(buf)
        }
    }
}

impl Write for WsAdapter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.socket.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}
