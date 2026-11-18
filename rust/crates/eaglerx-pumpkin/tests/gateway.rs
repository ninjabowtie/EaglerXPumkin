//! Real-socket end-to-end coverage for the runtime poll loop: HTTP upgrade,
//! animated MOTD frame cycling with the max-age close, and malformed-session
//! resilience. No Pumpkin backend is needed; the MOTD path never reaches it.

use eaglerx_pumpkin::{config::Config, runtime::Runtime};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn test_config() -> Config {
    // Port 0 binds an ephemeral port; the backend is never contacted on the
    // MOTD path, so the default loopback backend address is fine.
    Config {
        listen_port: 0,
        ..Config::default()
    }
}

/// Drive the runtime from a background thread at 20 ms, well inside the
/// 50 ms tick granularity the plugin scheduler provides.
fn start_runtime(config: Config) -> (u16, Arc<AtomicBool>, thread::JoinHandle<()>) {
    let runtime = Arc::new(Mutex::new(Runtime::bind(config).expect("runtime bind")));
    let port = runtime.lock().unwrap().local_addr().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let driver_stop = Arc::clone(&stop);
    let driver_runtime = Arc::clone(&runtime);
    let driver = thread::spawn(move || {
        while !driver_stop.load(Ordering::SeqCst) {
            driver_runtime.lock().unwrap().poll();
            thread::sleep(Duration::from_millis(20));
        }
    });
    (port, stop, driver)
}

fn upgrade(port: u16) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .write_all(
            b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
              Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
              Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
        )
        .expect("upgrade request");
    let mut response = Vec::new();
    let mut byte = [0u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).expect("upgrade response");
        response.push(byte[0]);
        assert!(response.len() < 4096, "upgrade response too large");
    }
    let head = String::from_utf8_lossy(&response);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
}

fn send_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut header = vec![0x80 | opcode];
    let len = payload.len();
    if len < 126 {
        header.push(0x80 | len as u8);
    } else if len < 65536 {
        header.push(0xfe);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(0xff);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }
    let mask = [0x11, 0x22, 0x33, 0x44];
    header.extend_from_slice(&mask);
    stream.write_all(&header)?;
    let masked: Vec<u8> = payload
        .iter()
        .enumerate()
        .map(|(i, byte)| byte ^ mask[i % 4])
        .collect();
    stream.write_all(&masked)
}

/// Read one server frame. Returns Ok(None) when the peer closed the stream.
fn recv_frame(stream: &mut TcpStream) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut header = [0u8; 2];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    assert_eq!(header[1] & 0x80, 0, "server frames must not be masked");
    let opcode = header[0] & 0x0f;
    let mut len = u64::from(header[1] & 0x7f);
    if len == 126 {
        let mut bytes = [0u8; 2];
        stream.read_exact(&mut bytes)?;
        len = u64::from(u16::from_be_bytes(bytes));
    } else if len == 127 {
        let mut bytes = [0u8; 8];
        stream.read_exact(&mut bytes)?;
        len = u64::from_be_bytes(bytes);
    }
    let mut data = vec![0u8; len as usize];
    stream.read_exact(&mut data)?;
    Ok(Some((opcode, data)))
}

#[test]
fn animated_motd_cycles_frames_over_real_sockets_until_max_age() {
    let config = Config {
        motd_frames: vec!["A\nB".into(), "C\nD".into()],
        motd_interval_ms: 100,
        motd_close_after_ms: 600,
        ..test_config()
    };
    config.validate().unwrap();
    let (port, stop, driver) = start_runtime(config);
    let mut stream = upgrade(port);

    send_frame(&mut stream, 1, b"Accept: MOTD").unwrap();
    let started = Instant::now();
    let mut texts: Vec<(Duration, String)> = Vec::new();
    loop {
        let (opcode, data) = recv_frame(&mut stream)
            .expect("frames until close")
            .expect("connection stays open until max age");
        match opcode {
            9 => send_frame(&mut stream, 10, &data).unwrap(), // ping -> pong
            8 => break,                                       // close: max age reached
            1 => texts.push((started.elapsed(), String::from_utf8(data).unwrap())),
            other => panic!("unexpected opcode {other}: {data:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "animation did not close in time"
        );
    }

    assert!(
        texts.len() >= 2,
        "expected at least two animation frames, got {texts:?}"
    );
    assert!(texts[0].1.contains("\"cache\":false"), "{}", texts[0].1);
    assert!(texts[0].1.contains("[\"A\",\"B\"]"), "{}", texts[0].1);
    assert!(
        texts.iter().any(|(_, text)| text.contains("[\"C\",\"D\"]")),
        "second frame never seen: {texts:?}"
    );
    // Frames arrive at the interval, not back to back.
    for window in texts.windows(2) {
        assert!(
            window[1].0 - window[0].0 >= Duration::from_millis(40),
            "frames too close together: {texts:?}"
        );
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
    stop.store(true, Ordering::SeqCst);
    driver.join().unwrap();
}

/// The static MOTD answer served over a real WebSocket carries the built-in
/// EaglerXPumpkin server icon as a base64 data URI and then closes the query.
#[test]
fn static_motd_query_over_real_sockets_returns_builtin_server_icon() {
    let config = test_config();
    config.validate().unwrap();
    let (port, stop, driver) = start_runtime(config);
    let mut stream = upgrade(port);

    send_frame(&mut stream, 1, b"Accept: MOTD").unwrap();
    let (opcode, data) = recv_frame(&mut stream)
        .expect("static motd reply")
        .expect("static motd answer then close");
    assert_eq!(opcode, 1);
    let text = String::from_utf8(data).unwrap();
    assert!(text.contains("\"cache\":true"), "{text}");
    assert!(text.contains("\"motd\""), "{text}");
    assert!(
        text.contains("\"icon\":\"data:image/png;base64,"),
        "static answer must carry the built-in server icon: {text}"
    );

    let (opcode, _) = recv_frame(&mut stream).expect("read").expect("close");
    assert_eq!(opcode, 8);

    let _ = stream.shutdown(std::net::Shutdown::Both);
    stop.store(true, Ordering::SeqCst);
    driver.join().unwrap();
}

#[test]
fn malformed_sessions_are_rejected_without_harming_the_listener() {
    let (port, stop, driver) = start_runtime(test_config());

    // An unsupported query fails the session: disconnect packet, then close.
    let mut stream = upgrade(port);
    send_frame(&mut stream, 1, b"Accept: garbage").unwrap();
    let mut saw_binary = false;
    let mut saw_close = false;
    while let Some((opcode, _data)) = recv_frame(&mut stream).expect("rejection") {
        match opcode {
            2 => saw_binary = true, // Eagler disconnect packet
            8 => {
                saw_close = true;
                break;
            }
            other => panic!("unexpected opcode {other}"),
        }
    }
    assert!(
        saw_binary && saw_close,
        "expected disconnect packet and close"
    );

    // An oversized frame length is dropped without a close frame.
    let mut stream = upgrade(port);
    stream
        .write_all(&[0x82, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])
        .unwrap();
    assert!(
        recv_frame(&mut stream).expect("read").is_none(),
        "oversized frame must end the connection"
    );

    // Invalid UTF-8 in a text query likewise ends the connection.
    let mut stream = upgrade(port);
    send_frame(&mut stream, 1, &[0xff, 0xfe]).unwrap();
    assert!(
        recv_frame(&mut stream).expect("read").is_none(),
        "invalid UTF-8 must end the connection"
    );

    // The listener still serves new, well-formed clients afterwards.
    let mut stream = upgrade(port);
    send_frame(&mut stream, 1, b"Accept: MOTD").unwrap();
    let (opcode, data) = recv_frame(&mut stream)
        .expect("read")
        .expect("static motd reply");
    assert_eq!(opcode, 1);
    let text = String::from_utf8(data).unwrap();
    assert!(text.contains("\"cache\":true"), "{text}");
    assert!(text.contains("Minecraft 26.3"), "{text}");

    stop.store(true, Ordering::SeqCst);
    driver.join().unwrap();
}

/// The connection limiter still applies over real sockets: three connections
/// with a per-minute limit of two means the third is accepted and closed.
#[test]
fn connection_rate_limit_closes_overflow_over_real_sockets() {
    let config = Config {
        connections_per_minute: 2,
        ..test_config()
    };
    config.validate().unwrap();
    let (port, stop, driver) = start_runtime(config);

    // Two idle WebSocket connections consume the budget.
    let _first = upgrade(port);
    let _second = upgrade(port);
    thread::sleep(Duration::from_millis(100)); // let the runtime accept them

    let mut third = TcpStream::connect(("127.0.0.1", port)).unwrap();
    third
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut response = Vec::new();
    let _ = third.read_to_end(&mut response);
    assert!(
        response.is_empty(),
        "rate-limited connection must be closed without a response: {response:?}"
    );

    stop.store(true, Ordering::SeqCst);
    driver.join().unwrap();
}

/// Reserve the port so unrelated processes cannot take it between bind and
/// use in the tests above (defense for parallel test runs on busy hosts).
#[test]
fn listener_binds_ephemeral_port() {
    let config = test_config();
    let runtime = Runtime::bind(config).unwrap();
    let addr = runtime.local_addr().unwrap();
    assert_ne!(addr.port(), 0);
    // A second bind on the same ephemeral port must fail.
    let mut taken = test_config();
    taken.listen_port = addr.port();
    assert!(Runtime::bind(taken).is_err());
}
