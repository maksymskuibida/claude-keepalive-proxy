use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Proxy {
    child: Child,
    port: u16,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_proxy() -> Proxy {
    let mut child = Command::new(env!("CARGO_BIN_EXE_keepalive-proxy"))
        .env("KEEPALIVE_PROXY_PORT", "0")
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    // Read stderr on a thread so a child that exits early (EOF) or hangs cannot block the test forever.
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut lines = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match lines.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(None);
                    return;
                }
                Ok(_) => {
                    if tx.send(Some(line.clone())).is_err() {
                        // Keep draining so the child never blocks on a full pipe.
                    }
                }
            }
        }
    });
    let mut proxy = Proxy { child, port: 0 };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Some(l)) => {
                if let Some(p) = l.trim().strip_prefix("listening 127.0.0.1:") {
                    proxy.port = p.parse().expect("bad port in listening line");
                    return proxy;
                }
            }
            Ok(None) => {
                let status = proxy.child.wait().ok();
                panic!("proxy exited before listening: {status:?}");
            }
            Err(_) => panic!("proxy did not report a listening port within 10s"),
        }
    }
}

fn start_echo() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut s in l.incoming().flatten() {
            thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = s.read(&mut buf) {
                    if n == 0 || s.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

fn read_head(s: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    String::from_utf8(head).unwrap()
}

#[test]
fn tunnel_connect_passes_bytes_both_ways() {
    let echo = start_echo();
    let proxy = start_proxy();
    let mut c = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(c, "CONNECT 127.0.0.1:{echo} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    assert!(read_head(&mut c).starts_with("HTTP/1.1 200"));
    c.write_all(b"hello tunnel").unwrap();
    let mut got = [0u8; 12];
    c.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"hello tunnel");
}

#[test]
fn tunnel_non_connect_gets_error_status() {
    let proxy = start_proxy();
    let mut c = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    c.write_all(b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n")
        .unwrap();
    let head = read_head(&mut c);
    let code: u16 = head.split(' ').nth(1).unwrap().parse().unwrap();
    assert!(code >= 400, "got {head}");
}

#[test]
fn tunnel_listens_on_loopback_only() {
    let proxy = start_proxy();
    let c = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
    assert!(c.peer_addr().unwrap().ip().is_loopback());
    let out = Command::new("lsof")
        .args([
            "-nP",
            "-a",
            "-p",
            &proxy.child.id().to_string(),
            "-iTCP",
            "-sTCP:LISTEN",
        ])
        .output();
    let o = out.expect("lsof must be available to inspect the listener");
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("LISTEN"), "no listener found: {text}");
    assert!(
        text.contains(&format!("127.0.0.1:{}", proxy.port)),
        "{text}"
    );
    assert!(!text.contains("*:"), "{text}");
}
