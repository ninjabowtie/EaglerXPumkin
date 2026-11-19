//! Exercise the actual native executable: a library test cannot detect a
//! missing tracing subscriber in main(). No Minecraft backend is contacted.
#![cfg(not(target_os = "wasi"))]
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Running {
    child: Child,
    directory: PathBuf,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn native_cli_emits_periodic_diagnostics_by_default() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("eagler-cli-{}-{unique}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let config = directory.join("eagler.toml");
    fs::write(
        &config,
        "listen_address = \"127.0.0.1\"\nlisten_port = 0\ndiagnostics_interval_secs = 1\n",
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_eaglerx-pumpkin"))
        .arg(config)
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut running = Running { child, directory };
    let stderr = running.child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut lines = Vec::new();
    let mut reported = false;
    while let Ok(line) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        reported = line.contains("Eagler gateway:");
        lines.push(line);
        if reported || Instant::now() >= deadline {
            break;
        }
    }
    drop(running);
    reader.join().unwrap();
    assert!(reported, "native diagnostics missing: {}", lines.join("\n"));
}
