//! TCP keepalive options.

use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_void};

extern "C" {
    fn setsockopt(fd: c_int, level: c_int, name: c_int, val: *const c_void, len: u32) -> c_int;
    #[cfg(test)]
    fn getsockopt(fd: c_int, level: c_int, name: c_int, val: *mut c_void, len: *mut u32) -> c_int;
}

#[cfg(target_os = "macos")]
mod c {
    use std::os::raw::c_int;
    pub const SOL_SOCKET: c_int = 0xffff;
    pub const SO_KEEPALIVE: c_int = 0x0008;
    pub const TCP_KEEPIDLE: c_int = 0x10;
    pub const TCP_KEEPINTVL: c_int = 0x101;
    pub const TCP_KEEPCNT: c_int = 0x102;
}
#[cfg(not(target_os = "macos"))]
mod c {
    use std::os::raw::c_int;
    pub const SOL_SOCKET: c_int = 1;
    pub const SO_KEEPALIVE: c_int = 9;
    pub const TCP_KEEPIDLE: c_int = 4;
    pub const TCP_KEEPINTVL: c_int = 5;
    pub const TCP_KEEPCNT: c_int = 6;
}
const IPPROTO_TCP: c_int = 6;
const TCP_NODELAY: c_int = 0x01;

pub fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn set(fd: c_int, level: c_int, name: c_int, val: c_int) {
    unsafe {
        setsockopt(fd, level, name, &val as *const c_int as *const c_void, 4);
    }
}

/// Enables keepalive with the given idle, interval and count, and disables Nagle.
pub fn tune(s: &TcpStream, idle: u32, intvl: u32, cnt: u32) {
    let fd = s.as_raw_fd();
    set(fd, c::SOL_SOCKET, c::SO_KEEPALIVE, 1);
    set(fd, IPPROTO_TCP, c::TCP_KEEPIDLE, idle as c_int);
    set(fd, IPPROTO_TCP, c::TCP_KEEPINTVL, intvl as c_int);
    set(fd, IPPROTO_TCP, c::TCP_KEEPCNT, cnt as c_int);
    set(fd, IPPROTO_TCP, TCP_NODELAY, 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

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
        assert_eq!(r, 0);
        val
    }

    #[test]
    fn sock_tune_sets_keepalive_options() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let s = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        tune(&s, 15, 10, 3);
        assert_ne!(get(&s, c::SOL_SOCKET, c::SO_KEEPALIVE), 0);
        assert_eq!(get(&s, IPPROTO_TCP, c::TCP_KEEPIDLE), 15);
        assert_eq!(get(&s, IPPROTO_TCP, c::TCP_KEEPINTVL), 10);
        assert_eq!(get(&s, IPPROTO_TCP, c::TCP_KEEPCNT), 3);
    }

    #[test]
    fn sock_env_default_and_override() {
        assert_eq!(env_u32("KEEPALIVE_TEST_UNSET_VAR", 7), 7);
        std::env::set_var("KEEPALIVE_TEST_SOCK_VAR", "42");
        assert_eq!(env_u32("KEEPALIVE_TEST_SOCK_VAR", 7), 42);
        std::env::set_var("KEEPALIVE_TEST_SOCK_VAR", "junk");
        assert_eq!(env_u32("KEEPALIVE_TEST_SOCK_VAR", 7), 7);
    }
}
