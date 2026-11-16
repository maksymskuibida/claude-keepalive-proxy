//! Probe (K-2 review): a proxy run the way the installed service runs it (stderr to a file,
//! no extra environment) must leave one closed-tunnel line per tunnel, so `report` counts it.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn k2_probe_report_counts_plain_tunnel_under_service_env() {
    let dir = std::env::temp_dir().join(format!("k2-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("proxy.log");
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_keepalive-proxy"))
        .arg("serve")
        .env_remove("KEEPALIVE_VERBOSE")
        .env("KEEPALIVE_PROXY_PORT", port.to_string())
        .stderr(Stdio::from(std::fs::File::create(&log).unwrap()))
        .spawn()
        .unwrap();
    let echo = TcpListener::bind("127.0.0.1:0").unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut s in echo.incoming().flatten() {
            let mut b = [0u8; 64];
            while let Ok(n) = s.read(&mut b) {
                if n == 0 || s.write_all(&b[..n]).is_err() {
                    break;
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut c = loop {
        if let Ok(c) = TcpStream::connect(("127.0.0.1", port)) {
            break c;
        }
        assert!(Instant::now() < deadline, "proxy did not listen");
        thread::sleep(Duration::from_millis(50));
    };
    write!(c, "CONNECT 127.0.0.1:{echo_port} HTTP/1.1\r\n\r\n").unwrap();
    let mut head = [0u8; 39];
    c.read_exact(&mut head).unwrap();
    c.write_all(b"ping").unwrap();
    let mut b = [0u8; 4];
    c.read_exact(&mut b).unwrap();
    drop(c);
    thread::sleep(Duration::from_millis(500));
    let _ = child.kill();
    let _ = child.wait();
    let out = Command::new(env!("CARGO_BIN_EXE_keepalive-proxy"))
        .arg("report")
        .arg(&log)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        text.contains("tunnels: 1"),
        "report output: {text:?}; log: {logged:?}"
    );
}
