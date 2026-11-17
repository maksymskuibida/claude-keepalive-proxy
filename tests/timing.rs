//! Timing tests for the keepalive schedule behind the "dead path cut in about 45 seconds" claim.
//!
//! Compiled only with `--features timing-tests`; the default `cargo test` does not run them.
//! Run: `cargo test --features timing-tests timing`.
//!
//! What these tests prove:
//! - the defaults (idle 15, interval 10, count 3) add up to 45 seconds, as the proxy itself reports;
//! - short settings from the environment reach the proxy and change the reported schedule;
//! - `tune` really sets the three keepalive options on a socket (read back with getsockopt), and the
//!   tunnel code calls it on both legs, upstream and client;
//! - a healthy but idle tunnel, whose peer still answers probes, is not cut by the short schedule.
//!
//! What they cannot prove:
//! - A peer that silently stops answering cannot be simulated on loopback without root: the kernel of
//!   the peer keeps answering keepalive probes, and dropping packets needs a firewall rule (pf or
//!   iptables). So there is no test here that a dead path is cut within the configured time.
//! - A real carrier-grade NAT path: the mapping timeouts, probe filtering and packet loss of a real
//!   carrier network exist only in the field. Nothing here shows that the probes keep a real NAT
//!   entry alive, or how a given carrier behaves.
#![cfg(feature = "timing-tests")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_void};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/sock.rs"]
mod sock;

extern "C" {
    fn getsockopt(fd: c_int, level: c_int, name: c_int, val: *mut c_void, len: *mut u32) -> c_int;
}

#[cfg(target_os = "macos")]
const SOL_SOCKET: c_int = 0xffff;
#[cfg(target_os = "macos")]
const SO_KEEPALIVE: c_int = 0x0008;
#[cfg(target_os = "macos")]
const TCP_KEEPIDLE: c_int = 0x10;
#[cfg(target_os = "macos")]
const TCP_KEEPINTVL: c_int = 0x101;
#[cfg(target_os = "macos")]
const TCP_KEEPCNT: c_int = 0x102;
#[cfg(not(target_os = "macos"))]
const SOL_SOCKET: c_int = 1;
#[cfg(not(target_os = "macos"))]
const SO_KEEPALIVE: c_int = 9;
#[cfg(not(target_os = "macos"))]
const TCP_KEEPIDLE: c_int = 4;
#[cfg(not(target_os = "macos"))]
const TCP_KEEPINTVL: c_int = 5;
#[cfg(not(target_os = "macos"))]
const TCP_KEEPCNT: c_int = 6;
const IPPROTO_TCP: c_int = 6;

fn get(s: &TcpStream, level: c_int, name: c_int) -> c_int {
    let mut val: c_int = -1;
    let mut len: u32 = 4;
    let r = unsafe {
        getsockopt(
            s.as_raw_fd(),
            level,
            name,
            &mut val as *mut c_int as *mut c_void,
            &mut len,
        )
    };
    assert_eq!(r, 0, "getsockopt failed");
    val
}

struct Proxy {
    child: Child,
    port: u16,
    banner: String,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the proxy with the given keepalive environment (None keeps the built-in default).
fn start_proxy(env: &[(&str, &str)]) -> Proxy {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_keepalive-proxy"));
    cmd.env("KEEPALIVE_PROXY_PORT", "0")
        .env_remove("KEEPALIVE_IDLE")
        .env_remove("KEEPALIVE_INTERVAL")
        .env_remove("KEEPALIVE_COUNT")
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut lines = BufReader::new(stderr);
    let mut port = 0u16;
    let mut banner = String::new();
    // The proxy prints the listening line, then the schedule line, before it accepts anything.
    for _ in 0..2 {
        let mut line = String::new();
        let n = lines.read_line(&mut line).unwrap();
        assert!(n > 0, "proxy exited before it reported its schedule");
        if let Some(p) = line.trim().strip_prefix("listening 127.0.0.1:") {
            port = p.parse().unwrap();
        } else {
            banner = line;
        }
    }
    // Keep draining so the child never blocks on a full pipe.
    thread::spawn(move || {
        let mut sink = String::new();
        while lines.read_line(&mut sink).map(|n| n > 0).unwrap_or(false) {
            sink.clear();
        }
    });
    assert_ne!(port, 0, "no listening line");
    Proxy {
        child,
        port,
        banner,
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
fn timing_default_schedule_adds_up_to_45_seconds() {
    let proxy = start_proxy(&[]);
    assert!(
        proxy.banner.contains("idle=15s")
            && proxy.banner.contains("interval=10s")
            && proxy.banner.contains("count=3"),
        "defaults are not 15/10/3: {}",
        proxy.banner
    );
    assert!(
        proxy.banner.contains("dead path detected in ~45s"),
        "{}",
        proxy.banner
    );
    // The sum is computed from the numbers the running proxy reported, not from constants here.
    let (idle, intvl, cnt) = (
        banner_number(&proxy.banner, "idle="),
        banner_number(&proxy.banner, "interval="),
        banner_number(&proxy.banner, "count="),
    );
    assert_eq!((idle, intvl, cnt), (15, 10, 3));
    assert_eq!(idle + intvl * cnt, 45);
    assert_eq!(banner_number(&proxy.banner, "detected in ~"), 45);
}

/// The number right after `key` in the proxy's schedule line.
fn banner_number(banner: &str, key: &str) -> u32 {
    let rest = banner.split(key).nth(1).expect("key missing in banner");
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().expect("no number after key")
}

#[test]
fn timing_env_overrides_change_the_schedule() {
    let proxy = start_proxy(&[
        ("KEEPALIVE_IDLE", "2"),
        ("KEEPALIVE_INTERVAL", "1"),
        ("KEEPALIVE_COUNT", "2"),
    ]);
    assert!(
        proxy.banner.contains("idle=2s")
            && proxy.banner.contains("interval=1s")
            && proxy.banner.contains("count=2"),
        "{}",
        proxy.banner
    );
    assert!(
        proxy.banner.contains("dead path detected in ~4s"),
        "{}",
        proxy.banner
    );
}

#[test]
fn timing_tune_sets_all_three_options_on_a_socket() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let s = TcpStream::connect(l.local_addr().unwrap()).unwrap();
    // Short settings, as they would come from the environment.
    sock::tune(&s, 2, 1, 2);
    assert_ne!(get(&s, SOL_SOCKET, SO_KEEPALIVE), 0);
    assert_eq!(get(&s, IPPROTO_TCP, TCP_KEEPIDLE), 2);
    assert_eq!(get(&s, IPPROTO_TCP, TCP_KEEPINTVL), 1);
    assert_eq!(get(&s, IPPROTO_TCP, TCP_KEEPCNT), 2);
}

#[test]
fn timing_tunnel_tunes_both_legs() {
    // The sockets of a running tunnel cannot be read from outside the proxy process, so this checks
    // the tunnel source: both the upstream and the client socket go through tune with the same schedule.
    // Whitespace and line breaks are removed first, so reformatting the source does not break this.
    let src: String =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tunnel.rs"))
            .unwrap()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
    assert!(
        src.contains("tune(&upstream,idle,intvl,cnt"),
        "upstream leg is not tuned"
    );
    assert!(
        src.contains("tune(&client,idle,intvl,cnt"),
        "client leg is not tuned"
    );
    let upstream_at = src.find("tune(&upstream").unwrap();
    let client_at = src.find("tune(&client").unwrap();
    let ack_at = src.find("200ConnectionEstablished").unwrap();
    assert!(client_at < ack_at, "legs must be tuned before data flows");
    assert!(upstream_at < ack_at, "legs must be tuned before data flows");
}

#[test]
fn timing_live_idle_tunnel_survives_past_the_dead_path_time() {
    // Schedule: idle 1 + interval 1 * count 2 = 3 seconds. The echo peer is alive and answers every
    // probe, so a tunnel idle for longer than that must still carry data afterwards.
    let echo = start_echo();
    let proxy = start_proxy(&[
        ("KEEPALIVE_IDLE", "1"),
        ("KEEPALIVE_INTERVAL", "1"),
        ("KEEPALIVE_COUNT", "2"),
    ]);
    let mut c = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(c, "CONNECT 127.0.0.1:{echo} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    assert!(read_head(&mut c).starts_with("HTTP/1.1 200"));
    let t0 = Instant::now();
    thread::sleep(Duration::from_secs(5));
    assert!(t0.elapsed() > Duration::from_secs(3));
    c.write_all(b"still here").unwrap();
    let mut got = [0u8; 10];
    c.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"still here");
}
