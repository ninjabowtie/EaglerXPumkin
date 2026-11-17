#[cfg(not(target_os = "wasi"))]
fn main() {
    use eaglerx_pumpkin::{config::Config, runtime::Runtime};
    use std::{path::PathBuf, time::Duration};
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| "eagler.toml".into());
    if args.next().is_some() {
        eprintln!("Usage: eaglerx-pumpkin [eagler.toml]");
        std::process::exit(2);
    }
    let result = Config::load(&path).and_then(|cfg| Runtime::bind(cfg).map_err(|e| e.to_string()));
    let mut runtime = match result {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("EaglerXPumpkin: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "EaglerXPumpkin listening on {} (config: {})",
        runtime.local_addr().unwrap(),
        path.display()
    );
    loop {
        runtime.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[cfg(target_os = "wasi")]
fn main() {
    eprintln!("Build the library target (--lib) for the Pumpkin plugin");
}
