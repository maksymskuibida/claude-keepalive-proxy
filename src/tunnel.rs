//! HTTP CONNECT tunnel on 127.0.0.1 with keepalive on both legs.
//!
//! Behind carrier-grade NAT an idle connection is silently forgotten and the client hangs for
//! minutes. Keepalive probes refresh the NAT entry; if the path is really dead the OS gives up
//! after IDLE + INTERVAL*COUNT seconds, the tunnel closes, and the client retries.

use crate::logline;
use crate::sock::{env_u32, tune};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Per-tunnel counters shared by both directions.
struct Stats {
    bytes_c2u: AtomicU64,
    bytes_u2c: AtomicU64,
    /// Longest silence (ms) seen waiting for data in either direction: how long the connection sat idle and survived.
    max_gap_ms: AtomicU64,
    /// Why the tunnel ended: the direction that finished first and how (eof / errno), plus its final wait.
    first_end: Mutex<Option<String>>,
}

fn pump(mut from: TcpStream, mut to: TcpStream, label: &'static str, stats: Arc<Stats>, up: bool) {
    let mut buf = vec![0u8; 64 * 1024];
    let end;
    loop {
        let t0 = Instant::now();
        match from.read(&mut buf) {
            Ok(0) => {
                let gap = t0.elapsed().as_millis() as u64;
                stats.max_gap_ms.fetch_max(gap, Ordering::Relaxed);
                end = format!("{label}:eof@{}s", gap / 1000);
                break;
            }
            Err(e) => {
                let gap = t0.elapsed().as_millis() as u64;
                end = format!(
                    "{label}:err{}@{}s",
                    e.raw_os_error().unwrap_or(-1),
                    gap / 1000
                );
                break;
            }
            Ok(n) => {
                stats
                    .max_gap_ms
                    .fetch_max(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
                let ctr = if up {
                    &stats.bytes_c2u
                } else {
                    &stats.bytes_u2c
                };
                ctr.fetch_add(n as u64, Ordering::Relaxed);
                if let Err(e) = to.write_all(&buf[..n]) {
                    end = format!("{label}:werr{}", e.raw_os_error().unwrap_or(-1));
                    break;
                }
            }
        }
    }
    if let Ok(mut f) = stats.first_end.lock() {
        if f.is_none() {
            *f = Some(end);
        }
    }
    let _ = to.shutdown(Shutdown::Both);
    let _ = from.shutdown(Shutdown::Both);
}

fn handle(mut client: TcpStream, idle: u32, intvl: u32, cnt: u32, verbose: bool) {
    let _ = client.set_read_timeout(Some(Duration::from_secs(15)));
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let end = loop {
        match client.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
        if let Some(p) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if head.len() > 16 * 1024 {
            return;
        }
    };
    let _ = client.set_read_timeout(None);
    let line = String::from_utf8_lossy(&head[..end])
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    let mut parts = line.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if !method.eq_ignore_ascii_case("CONNECT") {
        let _ = client.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    let t_start = std::time::Instant::now();
    let upstream = target.to_socket_addrs().ok().and_then(|addrs| {
        addrs
            .into_iter()
            .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(6)).ok())
    });
    let connect_ms = t_start.elapsed().as_millis();
    let Some(upstream) = upstream else {
        eprintln!("{} connect FAILED {target} after {connect_ms}ms", stamp());
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
        return;
    };
    if connect_ms > 1000 {
        eprintln!("{} SLOW connect {target} took {connect_ms}ms", stamp());
    }
    tune(&upstream, idle, intvl, cnt);
    tune(&client, idle, intvl, cnt);
    if client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .is_err()
    {
        return;
    }
    if head.len() > end && (&upstream).write_all(&head[end..]).is_err() {
        return;
    }
    if verbose {
        eprintln!("{} open {target} (connect {connect_ms}ms)", stamp());
    }
    let opened = std::time::Instant::now();
    let (c2, u2) = match (client.try_clone(), upstream.try_clone()) {
        (Ok(c), Ok(u)) => (c, u),
        _ => return,
    };
    let stats = Arc::new(Stats {
        bytes_c2u: AtomicU64::new(0),
        bytes_u2c: AtomicU64::new(0),
        max_gap_ms: AtomicU64::new(0),
        first_end: Mutex::new(None),
    });
    let s2 = Arc::clone(&stats);
    let t = thread::spawn(move || pump(c2, u2, "c2u", s2, true));
    pump(upstream, client, "u2c", Arc::clone(&stats), false);
    let _ = t.join();
    let end = stats
        .first_end
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    // errno 60 (ETIMEDOUT) on the upstream leg = keepalive probes went unanswered: the path was dead and we cut it
    // (without this proxy the client would have waited minutes). 54 = reset by peer.
    let dead_path = logline::is_dead_path(&end);
    if verbose || dead_path {
        eprintln!(
            "{}",
            logline::format(
                &stamp(),
                target,
                opened.elapsed().as_secs(),
                stats.bytes_c2u.load(Ordering::Relaxed),
                stats.bytes_u2c.load(Ordering::Relaxed),
                stats.max_gap_ms.load(Ordering::Relaxed) / 1000,
                &end,
            )
        );
    }
}

/// Seconds-resolution wall clock (UTC) without pulling in a date crate.
fn stamp() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{:02}:{:02}:{:02}Z", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

/// Runs the proxy until killed. Reads the KEEPALIVE_* environment variables. Returns an exit code.
pub fn serve() -> i32 {
    let port = env_u32("KEEPALIVE_PROXY_PORT", 18080);
    let idle = env_u32("KEEPALIVE_IDLE", 15);
    let intvl = env_u32("KEEPALIVE_INTERVAL", 10);
    let cnt = env_u32("KEEPALIVE_COUNT", 3);
    let verbose = env_u32("KEEPALIVE_VERBOSE", 0) == 1;
    let listener = match TcpListener::bind(("127.0.0.1", port as u16)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind 127.0.0.1:{port}: {e}");
            return 1;
        }
    };
    eprintln!(
        "keepalive proxy on 127.0.0.1:{port} idle={idle}s interval={intvl}s count={cnt} (dead path detected in ~{}s)",
        idle + intvl * cnt
    );
    for conn in listener.incoming().flatten() {
        thread::spawn(move || handle(conn, idle, intvl, cnt, verbose));
    }
    0
}
