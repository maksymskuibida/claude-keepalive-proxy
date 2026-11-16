//! Service manager: builds the LaunchAgent or LaunchDaemon plist and runs launchctl.
//! Contract (H-4): `args` holds only the arguments after the `service` subcommand.
//! Plist text is built by pure functions; launchctl runs behind the `Runner` trait.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Open-file limit (soft and hard) given to the service.
pub const FILE_LIMIT: u32 = 8192;
const THROTTLE_SECONDS: u32 = 3;
const NICE: i32 = -10;

/// Runs an external program: returns exit code and combined output.
pub trait Runner {
    fn run(&mut self, program: &str, args: &[String]) -> Result<(i32, String), String>;
}

struct RealRunner;

impl Runner for RealRunner {
    fn run(&mut self, program: &str, args: &[String]) -> Result<(i32, String), String> {
        let out = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run {program}: {e}"))?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok((out.status.code().unwrap_or(1), text))
    }
}

/// Everything read from the environment, so tests can supply their own.
#[derive(Debug, Clone)]
pub struct Env {
    pub exe: PathBuf,
    pub home: PathBuf,
    pub is_root: bool,
    pub sudo_user: Option<String>,
    /// `KEEPALIVE_PROXY_HOME`, when set (ignored for the system daemon).
    pub proxy_home: Option<PathBuf>,
    /// Folder for the system daemon plist (`/Library/LaunchDaemons`; tests use a temp folder).
    pub daemon_dir: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Uninstall,
    Status,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Opts {
    pub action: Action,
    pub system: bool,
    pub dry_run: bool,
    pub label: Option<String>,
}

pub fn parse_args(args: &[String]) -> Result<Opts, String> {
    let mut action = None;
    let mut system = false;
    let mut dry_run = false;
    let mut label = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "install" => action = Some(Action::Install),
            "uninstall" => action = Some(Action::Uninstall),
            "status" => action = Some(Action::Status),
            "--system" => system = true,
            "--dry-run" => dry_run = true,
            "--label" => {
                let v = it.next().ok_or("--label needs a value")?;
                if v.is_empty()
                    || !v
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
                {
                    return Err(format!("invalid label '{v}'"));
                }
                label = Some(v.clone());
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let action = action
        .ok_or("usage: service install|uninstall|status [--system] [--dry-run] [--label NAME]")?;
    Ok(Opts {
        action,
        system,
        dry_run,
        label,
    })
}

/// Default label derived from the program name.
pub fn default_label(exe: &Path) -> String {
    let name = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("keepalive-proxy");
    format!("local.{name}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Plist text. `user` is Some for the system daemon.
pub fn build_plist(label: &str, exe: &Path, log: &Path, user: Option<&str>) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    s.push_str("<plist version=\"1.0\">\n<dict>\n");
    s.push_str(&format!(
        "  <key>Label</key>\n  <string>{}</string>\n",
        xml_escape(label)
    ));
    s.push_str(&format!(
        "  <key>ProgramArguments</key>\n  <array>\n    <string>{}</string>\n    <string>serve</string>\n  </array>\n",
        xml_escape(&exe.to_string_lossy())
    ));
    s.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    s.push_str("  <key>KeepAlive</key>\n  <true/>\n");
    s.push_str(&format!(
        "  <key>ThrottleInterval</key>\n  <integer>{THROTTLE_SECONDS}</integer>\n"
    ));
    for key in ["SoftResourceLimits", "HardResourceLimits"] {
        s.push_str(&format!(
            "  <key>{key}</key>\n  <dict>\n    <key>NumberOfFiles</key>\n    <integer>{FILE_LIMIT}</integer>\n  </dict>\n"
        ));
    }
    s.push_str(&format!(
        "  <key>StandardErrorPath</key>\n  <string>{}</string>\n",
        xml_escape(&log.to_string_lossy())
    ));
    if let Some(u) = user {
        s.push_str(&format!(
            "  <key>UserName</key>\n  <string>{}</string>\n",
            xml_escape(u)
        ));
        s.push_str(&format!("  <key>Nice</key>\n  <integer>{NICE}</integer>\n"));
        s.push_str("  <key>ProcessType</key>\n  <string>Interactive</string>\n");
    }
    s.push_str("</dict>\n</plist>\n");
    s
}

/// Checks the sudo requirements for --system; returns the invoking user.
pub fn system_user(env: &Env) -> Result<String, String> {
    if !env.is_root {
        return Err("service --system needs root: run it with sudo. Nothing was changed.".into());
    }
    match env.sudo_user.as_deref() {
        Some(u) if !u.is_empty() && u != "root" => Ok(u.to_string()),
        _ => Err("service --system must be run through sudo by a normal user (SUDO_USER is missing or root). Nothing was changed.".into()),
    }
}

/// Parses `dscl . -read /Users/<name> NFSHomeDirectory` output.
pub fn parse_home(output: &str) -> Option<PathBuf> {
    let mut lines = output.lines();
    while let Some(l) = lines.next() {
        let Some(same) = l.trim().strip_prefix("NFSHomeDirectory:") else {
            continue;
        };
        let mut rest = same.trim();
        if rest.is_empty() {
            // A value with a space is printed on the next, indented line.
            match lines.next() {
                Some(n) if n.starts_with(' ') || n.starts_with('\t') => rest = n.trim(),
                _ => {}
            }
        }
        if rest.starts_with('/') && rest != "/" {
            return Some(PathBuf::from(rest));
        }
    }
    None
}

/// Looks up a user's home folder through the runner (system user database).
fn lookup_home(runner: &mut dyn Runner, user: &str) -> Result<PathBuf, String> {
    let args = vec![
        ".".to_string(),
        "-read".to_string(),
        format!("/Users/{user}"),
        "NFSHomeDirectory".to_string(),
    ];
    match runner.run("dscl", &args) {
        Ok((0, o)) => {
            parse_home(&o).ok_or_else(|| format!("cannot find the home folder of {user}"))
        }
        Ok((c, _)) => Err(format!(
            "cannot find the home folder of {user} (dscl exit {c})"
        )),
        Err(e) => Err(e),
    }
}

fn plist_path(label: &str, system: bool, home: &Path, daemon_dir: &Path) -> PathBuf {
    if system {
        daemon_dir.join(format!("{label}.plist"))
    } else {
        home.join("Library/LaunchAgents")
            .join(format!("{label}.plist"))
    }
}

/// Same folder and file name `report` reads: `$KEEPALIVE_PROXY_HOME/proxy.log`,
/// else `~/.keepalive-proxy/proxy.log`.
fn log_path(home: &Path, proxy_home: Option<&Path>) -> PathBuf {
    match proxy_home {
        Some(p) => p.join("proxy.log"),
        None => home.join(".keepalive-proxy").join("proxy.log"),
    }
}

fn describe(program: &str, args: &[String]) -> String {
    format!("{program} {}", args.join(" "))
}

/// Testable core. Writes messages to `out`; returns the exit code.
pub fn run_with(
    args: &[String],
    env: &Env,
    runner: &mut dyn Runner,
    out: &mut dyn FnMut(&str),
) -> i32 {
    let opts = match parse_args(args) {
        Ok(o) => o,
        Err(e) => {
            out(&format!("service: {e}"));
            return 2;
        }
    };
    let user = if opts.system {
        match system_user(env) {
            Ok(u) => Some(u),
            Err(e) => {
                out(&format!("service: {e}"));
                return 1;
            }
        }
    } else {
        None
    };
    // The daemon's state lives in the invoking user's home, not root's.
    // Only install needs it (state folder, log); the daemon plist path does not.
    let home = match &user {
        Some(u) if matches!(opts.action, Action::Install) => match lookup_home(runner, u) {
            Ok(h) => h,
            Err(e) => {
                out(&format!("service: {e}. Nothing was changed."));
                return 1;
            }
        },
        _ => env.home.clone(),
    };
    let label = opts
        .label
        .clone()
        .unwrap_or_else(|| default_label(&env.exe));
    let path = plist_path(&label, opts.system, &home, &env.daemon_dir);
    let path_s = path.to_string_lossy().into_owned();

    match opts.action {
        Action::Install => {
            let log = log_path(
                &home,
                if opts.system {
                    None
                } else {
                    env.proxy_home.as_deref()
                },
            );
            let plist = build_plist(&label, &env.exe, &log, user.as_deref());
            let chown = user.as_ref().and_then(|u| {
                log.parent().map(|d| {
                    vec![
                        "-R".to_string(),
                        u.clone(),
                        d.to_string_lossy().into_owned(),
                    ]
                })
            });
            let cmds = [
                ("launchctl", vec!["unload".to_string(), path_s.clone()]),
                (
                    "launchctl",
                    vec!["load".into(), "-w".into(), path_s.clone()],
                ),
            ];
            if opts.dry_run {
                out(&format!("would write {path_s}:\n{plist}"));
                for (p, a) in &cmds {
                    out(&format!("would run: {}", describe(p, a)));
                }
                if let Some(a) = &chown {
                    out(&format!("would run: {}", describe("chown", a)));
                }
                return 0;
            }
            if let Some(dir) = log.parent() {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    out(&format!("service: cannot create {}: {e}", dir.display()));
                    return 1;
                }
            }
            // Running as root: hand the state folder (and log) to the invoking user.
            if let Some(a) = &chown {
                match runner.run("chown", a) {
                    Ok((0, _)) => {}
                    Ok((c, o)) => {
                        out(&format!("service: chown failed ({c}): {}", o.trim()));
                        return 1;
                    }
                    Err(e) => {
                        out(&format!("service: {e}"));
                        return 1;
                    }
                }
            }
            if let Some(dir) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    out(&format!("service: cannot create {}: {e}", dir.display()));
                    return 1;
                }
            }
            if let Err(e) = std::fs::write(&path, plist) {
                out(&format!("service: cannot write {path_s}: {e}"));
                return 1;
            }
            // Unload first so a reinstall picks up the new plist; failure here is fine.
            let _ = runner.run(cmds[0].0, &cmds[0].1);
            match runner.run(cmds[1].0, &cmds[1].1) {
                Ok((0, _)) => {
                    out(&format!("installed and loaded {label} ({path_s})"));
                    0
                }
                Ok((c, o)) => {
                    out(&format!(
                        "service: launchctl load failed ({c}): {}",
                        o.trim()
                    ));
                    1
                }
                Err(e) => {
                    out(&format!("service: {e}"));
                    1
                }
            }
        }
        Action::Uninstall => {
            let unload = vec!["unload".to_string(), path_s.clone()];
            if opts.dry_run {
                out(&format!("would run: {}", describe("launchctl", &unload)));
                out(&format!("would delete {path_s}"));
                return 0;
            }
            if !path.exists() {
                out(&format!("{label} is not installed"));
                return 0;
            }
            if let Err(e) = runner.run("launchctl", &unload) {
                out(&format!("service: {e}"));
                return 1;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    out(&format!("unloaded and removed {label}"));
                    0
                }
                Err(e) => {
                    out(&format!("service: cannot delete {path_s}: {e}"));
                    1
                }
            }
        }
        Action::Status => {
            let list = vec!["list".to_string(), label.clone()];
            if opts.dry_run {
                out(&format!("would run: {}", describe("launchctl", &list)));
                return 0;
            }
            let installed = path.exists();
            let running = match runner.run("launchctl", &list) {
                Ok((0, o)) => o.contains("\"PID\""),
                _ => false,
            };
            out(&format!(
                "{label}: {}, {}",
                if installed {
                    "installed"
                } else {
                    "not installed"
                },
                if running { "running" } else { "not running" }
            ));
            0
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("keepalive-proxy"));
    let mut runner = RealRunner;
    let is_root = runner
        .run("id", &["-u".to_string()])
        .map(|(_, o)| o.trim() == "0")
        .unwrap_or(false);
    let env = Env {
        exe,
        home: std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default(),
        is_root,
        sudo_user: std::env::var("SUDO_USER").ok(),
        proxy_home: std::env::var_os("KEEPALIVE_PROXY_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        daemon_dir: PathBuf::from("/Library/LaunchDaemons"),
    };
    run_with(args, &env, &mut runner, &mut |m| println!("{m}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        calls: Vec<String>,
        reply: (i32, String),
    }

    impl Runner for Fake {
        fn run(&mut self, program: &str, args: &[String]) -> Result<(i32, String), String> {
            self.calls.push(describe(program, args));
            if program == "dscl" {
                return Ok((0, "NFSHomeDirectory: /fake/home/alice\n".into()));
            }
            Ok(self.reply.clone())
        }
    }

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn env(home: &Path, root: bool, sudo: Option<&str>) -> Env {
        Env {
            exe: PathBuf::from("/opt/tools/keepalive-proxy"),
            home: home.to_path_buf(),
            is_root: root,
            sudo_user: sudo.map(String::from),
            proxy_home: None,
            daemon_dir: home.join("daemons"),
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("svc-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn exec(args: &[&str], e: &Env, f: &mut Fake) -> (i32, String) {
        let mut text = String::new();
        let code = run_with(&v(args), e, f, &mut |m| {
            text.push_str(m);
            text.push('\n')
        });
        (code, text)
    }

    #[test]
    fn service_plist_has_keepalive_throttle_and_limits() {
        let p = build_plist("local.x", Path::new("/a/b"), Path::new("/l/p.log"), None);
        assert!(p.contains("<key>KeepAlive</key>\n  <true/>"));
        assert!(p.contains("<key>ThrottleInterval</key>\n  <integer>3</integer>"));
        assert!(p.contains("<key>SoftResourceLimits</key>"));
        assert!(p.contains("<key>HardResourceLimits</key>"));
        assert_eq!(p.matches("<integer>8192</integer>").count(), 2);
        assert!(p.contains("<key>StandardErrorPath</key>\n  <string>/l/p.log</string>"));
        assert!(p.contains("<string>/a/b</string>"));
        assert!(!p.contains("UserName") && !p.contains("Nice"));
    }

    #[test]
    fn service_plist_system_has_user_nice_interactive() {
        let p = build_plist("local.x", Path::new("/a/b"), Path::new("/l"), Some("alice"));
        assert!(p.contains("<key>UserName</key>\n  <string>alice</string>"));
        assert!(p.contains("<key>Nice</key>\n  <integer>-10</integer>"));
        assert!(p.contains("<key>ProcessType</key>\n  <string>Interactive</string>"));
    }

    #[test]
    fn service_file_limit_covers_2000_tunnels() {
        // Each tunnel holds 4 descriptors (client and upstream, two each).
        let fds_per_tunnel = std::hint::black_box(4);
        assert!(FILE_LIMIT / fds_per_tunnel >= 2000);
    }

    #[test]
    fn service_default_label_from_program_name() {
        assert_eq!(default_label(Path::new("/x/my-proxy")), "local.my-proxy");
        assert!(parse_args(&v(&["install", "--label", "bad label"])).is_err());
    }

    #[test]
    fn service_system_refused_without_root() {
        let h = tmp("noroot");
        let mut f = Fake {
            calls: vec![],
            reply: (0, String::new()),
        };
        let (c, t) = exec(
            &["install", "--system"],
            &env(&h, false, Some("alice")),
            &mut f,
        );
        assert_eq!(c, 1);
        assert!(t.contains("sudo"));
        assert!(f.calls.is_empty());
    }

    #[test]
    fn service_system_refused_without_sudo_user_or_as_root() {
        let h = tmp("nouser");
        for sudo in [None, Some("root"), Some("")] {
            let mut f = Fake {
                calls: vec![],
                reply: (0, String::new()),
            };
            let (c, t) = exec(
                &["install", "--system", "--dry-run"],
                &env(&h, true, sudo),
                &mut f,
            );
            assert_eq!(c, 1);
            assert!(t.contains("SUDO_USER"));
            assert!(f.calls.is_empty());
        }
    }

    #[test]
    fn service_system_dry_run_prints_daemon_plist_for_sudo_user() {
        let h = tmp("sysdry");
        let mut f = Fake {
            calls: vec![],
            reply: (0, String::new()),
        };
        let (c, t) = exec(
            &["install", "--system", "--dry-run"],
            &env(&h, true, Some("alice")),
            &mut f,
        );
        assert_eq!(c, 0);
        assert!(t.contains("daemons/local.keepalive-proxy.plist"));
        assert!(t.contains("<string>alice</string>"));
        assert!(t.contains("<integer>-10</integer>"));
        assert!(t.contains("would run: launchctl load -w"));
        assert!(t.contains("/fake/home/alice/.keepalive-proxy/proxy.log"));
        assert!(t.contains("would run: chown -R alice /fake/home/alice/.keepalive-proxy"));
        // Only the read-only user lookup ran.
        assert!(f.calls.iter().all(|c| c.starts_with("dscl ")));
        assert!(!h.join("daemons").exists());
    }

    #[test]
    fn service_system_install_chowns_state_folder_to_sudo_user() {
        let h = tmp("syschown");
        // Pretend the invoking user's home is inside the temp folder.
        struct Home(Fake, PathBuf);
        impl Runner for Home {
            fn run(&mut self, p: &str, a: &[String]) -> Result<(i32, String), String> {
                if p == "dscl" {
                    self.0.calls.push(describe(p, a));
                    return Ok((0, format!("NFSHomeDirectory: {}\n", self.1.display())));
                }
                self.0.run(p, a)
            }
        }
        let uh = h.join("alice-home");
        let mut r = Home(
            Fake {
                calls: vec![],
                reply: (0, String::new()),
            },
            uh.clone(),
        );
        let mut text = String::new();
        let c = run_with(
            &v(&["install", "--system"]),
            &env(&h, true, Some("alice")),
            &mut r,
            &mut |m| text.push_str(m),
        );
        assert_eq!(c, 0, "{text}");
        let state = uh.join(".keepalive-proxy");
        assert!(state.is_dir());
        let chown = format!("chown -R alice {}", state.display());
        let calls = &r.0.calls;
        let ci = calls.iter().position(|c| *c == chown).expect("chown ran");
        let li = calls
            .iter()
            .position(|c| c.starts_with("launchctl load"))
            .unwrap();
        assert!(ci < li);
        assert!(h.join("daemons/local.keepalive-proxy.plist").exists());
        let plist = std::fs::read_to_string(h.join("daemons/local.keepalive-proxy.plist")).unwrap();
        assert!(plist.contains(&format!("{}/proxy.log", state.display())));
    }

    #[test]
    fn service_system_install_stops_when_home_lookup_fails() {
        let h = tmp("syshomefail");
        struct Bad(Vec<String>);
        impl Runner for Bad {
            fn run(&mut self, p: &str, a: &[String]) -> Result<(i32, String), String> {
                self.0.push(describe(p, a));
                Ok((1, String::new()))
            }
        }
        let mut r = Bad(vec![]);
        let mut text = String::new();
        let c = run_with(
            &v(&["install", "--system"]),
            &env(&h, true, Some("alice")),
            &mut r,
            &mut |m| text.push_str(m),
        );
        assert_eq!(c, 1);
        assert!(text.contains("home folder"));
        assert_eq!(r.0.len(), 1);
        assert!(!h.join("daemons").exists());
    }

    #[test]
    fn service_home_parsed_from_user_database_output() {
        assert_eq!(
            parse_home("NFSHomeDirectory: /Users/bob\n"),
            Some(PathBuf::from("/Users/bob"))
        );
        assert_eq!(parse_home("NFSHomeDirectory: relative"), None);
        assert_eq!(parse_home("nothing"), None);
    }

    #[test]
    fn service_home_parsed_from_next_indented_line() {
        assert_eq!(
            parse_home("NFSHomeDirectory:\n /Users/made up/bob\n"),
            Some(PathBuf::from("/Users/made up/bob"))
        );
        assert_eq!(parse_home("NFSHomeDirectory:\nnot-indented"), None);
    }

    #[test]
    fn service_system_uninstall_skips_home_lookup() {
        let h = tmp("sysuninst");
        struct Bad(Vec<String>);
        impl Runner for Bad {
            fn run(&mut self, p: &str, a: &[String]) -> Result<(i32, String), String> {
                self.0.push(describe(p, a));
                Ok((1, String::new()))
            }
        }
        let mut r = Bad(vec![]);
        let mut text = String::new();
        run_with(
            &v(&["uninstall", "--system"]),
            &env(&h, true, Some("alice")),
            &mut r,
            &mut |m| text.push_str(m),
        );
        assert!(!text.contains("home folder"));
        assert!(r.0.iter().all(|c| !c.starts_with("dscl")));
    }

    #[test]
    fn service_install_writes_plist_and_loads() {
        let h = tmp("install");
        let mut f = Fake {
            calls: vec![],
            reply: (0, String::new()),
        };
        let (c, _) = exec(
            &["install", "--label", "local.demo"],
            &env(&h, false, None),
            &mut f,
        );
        assert_eq!(c, 0);
        let p = h.join("Library/LaunchAgents/local.demo.plist");
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("<string>local.demo</string>"));
        assert!(text.contains(".keepalive-proxy/proxy.log"));
        assert_eq!(
            f.calls.last().unwrap(),
            &format!("launchctl load -w {}", p.display())
        );
        // The LaunchAgent runs as the user already: no ownership step, no user lookup.
        assert!(f
            .calls
            .iter()
            .all(|c| !c.starts_with("chown") && !c.starts_with("dscl")));
    }

    #[test]
    fn service_install_reports_load_failure() {
        let h = tmp("loadfail");
        let mut f = Fake {
            calls: vec![],
            reply: (5, "boom".into()),
        };
        let (c, t) = exec(&["install"], &env(&h, false, None), &mut f);
        assert_eq!(c, 1);
        assert!(t.contains("boom"));
    }

    #[test]
    fn service_uninstall_unloads_and_deletes_plist() {
        let h = tmp("uninstall");
        let mut f = Fake {
            calls: vec![],
            reply: (0, String::new()),
        };
        let e = env(&h, false, None);
        exec(&["install", "--label", "local.demo"], &e, &mut f);
        f.calls.clear();
        let p = h.join("Library/LaunchAgents/local.demo.plist");
        let (c, _) = exec(&["uninstall", "--label", "local.demo"], &e, &mut f);
        assert_eq!(c, 0);
        assert_eq!(f.calls, vec![format!("launchctl unload {}", p.display())]);
        assert!(!p.exists());
    }

    #[test]
    fn service_dry_run_changes_nothing() {
        let h = tmp("dry");
        let mut f = Fake {
            calls: vec![],
            reply: (0, String::new()),
        };
        let (c, t) = exec(&["install", "--dry-run"], &env(&h, false, None), &mut f);
        assert_eq!(c, 0);
        assert!(t.contains("<plist"));
        assert!(f.calls.is_empty());
        assert!(!h.join("Library").exists());
    }

    #[test]
    fn service_status_reports_installed_and_running() {
        let h = tmp("status");
        let e = env(&h, false, None);
        let mut f = Fake {
            calls: vec![],
            reply: (1, String::new()),
        };
        let (_, t) = exec(&["status"], &e, &mut f);
        assert!(t.contains("not installed, not running"));
        exec(&["install"], &e, &mut f);
        f.reply = (0, "{\n\t\"PID\" = 123;\n}".into());
        let (c, t) = exec(&["status"], &e, &mut f);
        assert_eq!(c, 0);
        assert!(t.contains("installed, running"));
        f.reply = (0, "{\n\t\"LastExitStatus\" = 0;\n}".into());
        let (_, t) = exec(&["status"], &e, &mut f);
        assert!(t.contains("installed, not running"));
    }
}
