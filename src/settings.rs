//! Claude Code settings editor: `on`, `off` and `status`.
//! Contract (H-3): `args[0]` is the command word (`on`, `off` or `status`), followed by the remaining arguments.
//!
//! `on` sets `HTTPS_PROXY` and `NO_PROXY` in the `env` block of the settings file; `off` removes exactly those two
//! entries. The file is parsed first and never touched if it is not valid JSON; writes go to a temporary file in the
//! same folder and are renamed into place.

use crate::json::{self, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const KEY_PROXY: &str = "HTTPS_PROXY";
const KEY_NO_PROXY: &str = "NO_PROXY";
const DEFAULT_PORT: u16 = 18080;
const DEFAULT_NO_PROXY: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "::1",
    "github.com",
    "gitlab.com",
    "google.com",
    "registry.npmjs.org",
    "npmjs.org",
];

/// Where the settings file lives: `CLAUDE_SETTINGS` if set and non-empty, else `<home>/.claude/settings.json`.
fn settings_path(override_path: Option<&str>, home: Option<&str>) -> Result<PathBuf, String> {
    match override_path {
        Some(p) if !p.is_empty() => Ok(PathBuf::from(p)),
        _ => match home {
            Some(h) if !h.is_empty() => Ok(Path::new(h).join(".claude").join("settings.json")),
            _ => Err("cannot find the settings file: set CLAUDE_SETTINGS or HOME".to_string()),
        },
    }
}

const USAGE: &str = "use --port N, --no-proxy HOSTS or --force (overwrite an existing HTTPS_PROXY or NO_PROXY that differs)";

struct Options {
    port: u16,
    extra_hosts: Vec<String>,
    force: bool,
}

fn parse_options(args: &[String], default_port: u16) -> Result<Options, String> {
    let mut opts = Options {
        port: default_port,
        extra_hosts: Vec::new(),
        force: false,
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (a, None),
        };
        match name {
            "--force" if inline.is_none() => opts.force = true,
            "--port" | "--no-proxy" => {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i)
                            .cloned()
                            .ok_or_else(|| format!("{name} needs a value"))?
                    }
                };
                if name == "--port" {
                    opts.port = value
                        .parse::<u16>()
                        .ok()
                        .filter(|p| *p != 0)
                        .ok_or_else(|| format!("invalid port '{value}'"))?;
                } else {
                    for h in value.split(',').map(str::trim).filter(|h| !h.is_empty()) {
                        if h.chars().any(|c| c.is_whitespace() || c == '"') {
                            return Err(format!("invalid host '{h}'"));
                        }
                        opts.extra_hosts.push(h.to_string());
                    }
                }
            }
            _ => return Err(format!("unknown option '{a}'; {USAGE}")),
        }
        i += 1;
    }
    Ok(opts)
}

fn no_proxy_value(extra: &[String]) -> String {
    let mut hosts: Vec<&str> = DEFAULT_NO_PROXY.to_vec();
    for h in extra {
        if !hosts.contains(&h.as_str()) {
            hosts.push(h);
        }
    }
    hosts.join(",")
}

/// Follow symlinks (also dangling ones) to the path that should really be read or written.
fn resolve_link(path: &Path) -> Result<PathBuf, String> {
    let mut cur = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                let t = fs::read_link(&cur)
                    .map_err(|e| format!("cannot read link {}: {e}", cur.display()))?;
                cur = if t.is_absolute() {
                    t
                } else {
                    match cur.parent() {
                        Some(p) => p.join(&t),
                        None => t,
                    }
                };
            }
            _ => return Ok(cur),
        }
    }
    Err(format!("too many symbolic links at {}", path.display()))
}

/// Read and parse the settings file. `Ok(None)` when it does not exist. A leading byte-order mark is stripped
/// (the JSON parser refuses one).
fn load(path: &Path) -> Result<Option<(Value, bool)>, String> {
    let resolved = resolve_link(path)?;
    let bytes = match fs::read(&resolved) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("{} is not valid UTF-8; left untouched", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value =
        json::parse(text).map_err(|e| format!("{}: {e}; left untouched", path.display()))?;
    if !matches!(value, Value::Object(_)) {
        return Err(format!(
            "{} is not a JSON object; left untouched",
            path.display()
        ));
    }
    let newline = text.ends_with('\n');
    Ok(Some((value, newline)))
}

static TMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Atomic write: temp file in the same folder, flushed, then renamed over the target.
fn write_atomic(path: &Path, value: &Value, newline: bool) -> Result<(), String> {
    // Write through a symlink (even a dangling one) rather than replacing it.
    let target = resolve_link(path)?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".to_string());
    let mut text = value.write_pretty();
    if newline {
        text.push('\n');
    }
    let perms = fs::metadata(&target).ok().map(|m| m.permissions());
    // Exclusive create (never follows or reuses an existing path), permissions set before any data is written.
    let mut attempt = 0;
    let (mut f, tmp) = loop {
        let tmp = dir.join(format!(
            ".{name}.tmp{}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut oo = fs::OpenOptions::new();
        oo.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            oo.mode(0o600);
        }
        match oo.open(&tmp) {
            Ok(f) => break (f, tmp),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 10 => attempt += 1,
            Err(e) => return Err(format!("cannot write {}: {e}", target.display())),
        }
    };
    let result = (|| -> std::io::Result<()> {
        if let Some(p) = perms {
            f.set_permissions(p)?;
        }
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, &target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot write {}: {e}", target.display()));
    }
    Ok(())
}

fn env_block<'a>(root: &'a mut Value, path: &Path) -> Result<&'a mut Value, String> {
    if root.get("env").is_none() {
        root.set("env", Value::Object(Vec::new()));
    }
    match root.get_mut("env") {
        Some(v @ Value::Object(_)) => Ok(v),
        _ => Err(format!(
            "'env' in {} is not an object; left untouched",
            path.display()
        )),
    }
}

/// True when the env block holds the entries this tool writes: HTTPS_PROXY is `http://127.0.0.1:<port>` and
/// NO_PROXY starts with the default host list.
fn is_ours(env: Option<&Value>) -> bool {
    let Some(env) = env else {
        return false;
    };
    let (Some(Value::String(p)), Some(Value::String(np))) =
        (env.get(KEY_PROXY), env.get(KEY_NO_PROXY))
    else {
        return false;
    };
    let port_ok = p
        .strip_prefix("http://127.0.0.1:")
        .and_then(|r| r.parse::<u16>().ok())
        .is_some();
    port_ok && np.starts_with(&no_proxy_value(&[]))
}

fn do_on(path: &Path, opts: &Options) -> Result<String, String> {
    let (mut root, newline) = load(path)?.unwrap_or_else(|| (Value::Object(Vec::new()), true));
    let env = env_block(&mut root, path)?;
    let ours = is_ours(Some(&*env));
    let proxy = format!("http://127.0.0.1:{}", opts.port);
    let no_proxy = no_proxy_value(&opts.extra_hosts);
    if !opts.force && !ours {
        for (key, new) in [(KEY_PROXY, &proxy), (KEY_NO_PROXY, &no_proxy)] {
            if let Some(old) = env.get(key) {
                if old != &Value::String(new.clone()) {
                    return Err(format!(
                        "{key} is already set in {} to a different value; left untouched (use --force to overwrite it)",
                        path.display()
                    ));
                }
            }
        }
    }
    env.set(KEY_PROXY, Value::String(proxy.clone()));
    env.set(KEY_NO_PROXY, Value::String(no_proxy.clone()));
    write_atomic(path, &root, newline)?;
    Ok(format!(
        "on: {KEY_PROXY}={proxy}, {KEY_NO_PROXY}={no_proxy} ({})",
        path.display()
    ))
}

fn do_off(path: &Path) -> Result<String, String> {
    let Some((mut root, newline)) = load(path)? else {
        return Ok(format!(
            "off: {} does not exist, nothing to do",
            path.display()
        ));
    };
    let mut removed = false;
    let mut foreign = false;
    match root.get_mut("env") {
        Some(env @ Value::Object(_)) => {
            let present = env.get(KEY_PROXY).is_some() || env.get(KEY_NO_PROXY).is_some();
            if is_ours(Some(&*env)) {
                env.remove(KEY_PROXY);
                env.remove(KEY_NO_PROXY);
                removed = true;
            } else {
                foreign = present;
            }
        }
        Some(_) => {
            return Err(format!(
                "'env' in {} is not an object; left untouched",
                path.display()
            ))
        }
        None => {}
    }
    if !removed {
        if foreign {
            return Ok(format!(
                "off: {KEY_PROXY}/{KEY_NO_PROXY} were set by something else, left untouched ({})",
                path.display()
            ));
        }
        return Ok(format!("off: already off ({})", path.display()));
    }
    if matches!(root.get("env"), Some(Value::Object(i)) if i.is_empty()) {
        root.remove("env");
    }
    if matches!(&root, Value::Object(i) if i.is_empty()) {
        // Nothing else is left in the file: remove it (it was most likely created by `on`).
        let target = resolve_link(path)?;
        fs::remove_file(&target).map_err(|e| format!("cannot remove {}: {e}", target.display()))?;
        return Ok(format!(
            "off: removed the proxy entries and the empty settings file ({})",
            path.display()
        ));
    }
    write_atomic(path, &root, newline)?;
    Ok(format!(
        "off: removed the proxy entries ({})",
        path.display()
    ))
}

fn do_status(path: &Path) -> Result<String, String> {
    let root = load(path)?;
    let env = root.as_ref().and_then(|(r, _)| r.get("env"));
    let text = |k: &str| match env.and_then(|e| e.get(k)) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    let has_proxy = env.and_then(|e| e.get(KEY_PROXY)).is_some();
    Ok(if is_ours(env) {
        let p = text(KEY_PROXY).unwrap_or_default();
        let np = text(KEY_NO_PROXY).unwrap_or_else(|| "(not set)".to_string());
        format!(
            "on: {KEY_PROXY}={p}, {KEY_NO_PROXY}={np} ({})",
            path.display()
        )
    } else if has_proxy {
        format!("set by something else: {KEY_PROXY} ({})", path.display())
    } else {
        format!("off ({})", path.display())
    })
}

pub fn run(args: &[String]) -> i32 {
    let Some(cmd) = args.first() else {
        eprintln!("settings: missing command (on, off or status)");
        return 2;
    };
    let rest = &args[1..];
    let env_path = std::env::var("CLAUDE_SETTINGS").ok();
    let home = std::env::var("HOME").ok();
    let outcome =
        settings_path(env_path.as_deref(), home.as_deref()).and_then(|path| match cmd.as_str() {
            "on" => {
                let default_port = std::env::var("KEEPALIVE_PROXY_PORT")
                    .ok()
                    .and_then(|p| p.parse::<u16>().ok())
                    .filter(|p| *p != 0)
                    .unwrap_or(DEFAULT_PORT);
                let opts = parse_options(rest, default_port)?;
                do_on(&path, &opts)
            }
            "off" | "status" if !rest.is_empty() => {
                Err(format!("unexpected argument '{}'", rest[0]))
            }
            "off" => do_off(&path),
            "status" => do_status(&path),
            other => Err(format!("unknown command '{other}'")),
        });
    match outcome {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => {
            eprintln!("settings: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "kp-settings-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn file(&self) -> PathBuf {
            self.0.join("settings.json")
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn opts() -> Options {
        Options {
            port: 18080,
            extra_hosts: vec![],
            force: false,
        }
    }

    fn read(p: &Path) -> Value {
        json::parse(&fs::read_to_string(p).unwrap()).unwrap()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn settings_on_creates_missing_file_and_folder() {
        let t = Tmp::new("create");
        let f = t.0.join("nested").join("settings.json");
        do_on(&f, &opts()).unwrap();
        let v = read(&f);
        let env = v.get("env").unwrap();
        assert_eq!(
            env.get("HTTPS_PROXY"),
            Some(&Value::String("http://127.0.0.1:18080".into()))
        );
        let Some(Value::String(np)) = env.get("NO_PROXY") else {
            panic!("no NO_PROXY")
        };
        for h in [
            "localhost",
            "127.0.0.1",
            "github.com",
            "gitlab.com",
            "google.com",
            "registry.npmjs.org",
        ] {
            assert!(np.split(',').any(|x| x == h), "missing {h}");
        }
        assert!(fs::read_to_string(&f).unwrap().ends_with("}\n"));
    }

    #[test]
    fn settings_on_adds_extra_hosts_and_port() {
        let t = Tmp::new("extra");
        let o = parse_options(
            &s(&[
                "--port",
                "9000",
                "--no-proxy",
                "a.example,b.example",
                "--no-proxy=github.com",
            ]),
            1,
        )
        .unwrap();
        do_on(&t.file(), &o).unwrap();
        let v = read(&t.file());
        let env = v.get("env").unwrap();
        assert_eq!(
            env.get("HTTPS_PROXY"),
            Some(&Value::String("http://127.0.0.1:9000".into()))
        );
        let Some(Value::String(np)) = env.get("NO_PROXY") else {
            panic!()
        };
        assert!(np.ends_with("a.example,b.example"));
        assert_eq!(np.matches("github.com").count(), 1);
        assert!(parse_options(&s(&["--port", "0"]), 1).is_err());
        assert!(parse_options(&s(&["--port", "70000"]), 1).is_err());
        assert!(parse_options(&s(&["--bogus"]), 1).is_err());
        assert!(parse_options(&s(&["--no-proxy"]), 1).is_err());
    }

    #[test]
    fn settings_on_keeps_other_keys_and_order() {
        let t = Tmp::new("keep");
        fs::write(
            t.file(),
            r#"{"theme":"dark","env":{"FOO":"1","NO_PROXY":"old"},"permissions":{"allow":["x"]},"n":1.50}"#,
        )
        .unwrap();
        do_on(
            &t.file(),
            &Options {
                force: true,
                ..opts()
            },
        )
        .unwrap();
        let v = read(&t.file());
        let Value::Object(items) = &v else { panic!() };
        let keys: Vec<&str> = items.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["theme", "env", "permissions", "n"]);
        let Some(Value::Object(env)) = v.get("env") else {
            panic!()
        };
        let ek: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(ek, ["FOO", "NO_PROXY", "HTTPS_PROXY"]);
        assert_eq!(v.get("n"), Some(&Value::Number("1.50".into())));
        assert_eq!(v.get("theme"), Some(&Value::String("dark".into())));
    }

    #[test]
    fn settings_off_removes_exactly_two_entries() {
        let t = Tmp::new("off");
        fs::write(
            t.file(),
            format!(
                r#"{{"a":1,"env":{{"FOO":"1","HTTPS_PROXY":"http://127.0.0.1:1","NO_PROXY":"{}","BAR":"2"}},"z":[]}}"#,
                no_proxy_value(&[])
            ),
        )
        .unwrap();
        do_off(&t.file()).unwrap();
        let v = read(&t.file());
        assert_eq!(
            v,
            json::parse(r#"{"a":1,"env":{"FOO":"1","BAR":"2"},"z":[]}"#).unwrap()
        );
        // Second off changes nothing.
        let before = fs::read(t.file()).unwrap();
        do_off(&t.file()).unwrap();
        assert_eq!(fs::read(t.file()).unwrap(), before);
        // Missing file: nothing created.
        let missing = t.0.join("none.json");
        do_off(&missing).unwrap();
        assert!(!missing.exists());
    }

    #[test]
    fn settings_on_then_off_round_trip() {
        let t = Tmp::new("round");
        fs::write(t.file(), r#"{"k":"v"}"#).unwrap();
        do_on(&t.file(), &opts()).unwrap();
        assert!(do_status(&t.file()).unwrap().starts_with("on:"));
        do_off(&t.file()).unwrap();
        assert!(do_status(&t.file()).unwrap().starts_with("off"));
        assert_eq!(read(&t.file()).get("k"), Some(&Value::String("v".into())));
    }

    #[test]
    fn settings_round_trip_is_byte_exact_without_env() {
        let t = Tmp::new("bytes");
        for body in ["{\n  \"k\": \"v\"\n}\n", "{\n  \"k\": \"v\"\n}"] {
            fs::write(t.file(), body).unwrap();
            do_on(&t.file(), &opts()).unwrap();
            do_off(&t.file()).unwrap();
            assert_eq!(fs::read_to_string(t.file()).unwrap(), body);
        }
    }

    #[test]
    fn settings_round_trip_missing_file_stays_missing() {
        let t = Tmp::new("missing");
        do_on(&t.file(), &opts()).unwrap();
        assert!(t.file().exists());
        do_off(&t.file()).unwrap();
        assert!(!t.file().exists());
    }

    #[test]
    fn settings_round_trip_keeps_other_env_entries_in_order() {
        let t = Tmp::new("envkeep");
        let body = "{\n  \"env\": {\n    \"B\": \"2\",\n    \"A\": \"1\"\n  },\n  \"z\": 1\n}\n";
        fs::write(t.file(), body).unwrap();
        do_on(&t.file(), &opts()).unwrap();
        do_off(&t.file()).unwrap();
        assert_eq!(fs::read_to_string(t.file()).unwrap(), body);
    }

    #[test]
    fn settings_on_replaces_own_entries_without_force() {
        let t = Tmp::new("replace");
        do_on(&t.file(), &opts()).unwrap();
        let o = Options {
            port: 9000,
            extra_hosts: s(&["a.example"]),
            force: false,
        };
        do_on(&t.file(), &o).unwrap();
        let v = read(&t.file());
        let env = v.get("env").unwrap();
        assert_eq!(
            env.get("HTTPS_PROXY"),
            Some(&Value::String("http://127.0.0.1:9000".into()))
        );
        assert!(do_status(&t.file()).unwrap().starts_with("on:"));
    }

    #[test]
    fn settings_users_own_local_proxy_is_left_alone() {
        let t = Tmp::new("localproxy");
        let body = r#"{"env":{"HTTPS_PROXY":"http://127.0.0.1:3128","NO_PROXY":"mine.example"}}"#;
        fs::write(t.file(), body).unwrap();
        let msg = do_off(&t.file()).unwrap();
        assert!(msg.contains("something else"), "{msg}");
        assert_eq!(fs::read_to_string(t.file()).unwrap(), body);
        assert!(do_status(&t.file())
            .unwrap()
            .starts_with("set by something else"));
        let err = do_on(&t.file(), &opts()).unwrap_err();
        assert!(err.contains("--force"), "{err}");
        assert_eq!(fs::read_to_string(t.file()).unwrap(), body);
    }

    #[test]
    fn settings_invalid_file_is_untouched() {
        let t = Tmp::new("invalid");
        for bad in [
            "{not json",
            "",
            "[1,2]",
            r#"{"env":"str"}"#,
            r#"{"env":[]}"#,
        ] {
            fs::write(t.file(), bad).unwrap();
            assert!(do_on(&t.file(), &opts()).is_err(), "on accepted {bad:?}");
            assert!(
                do_off(&t.file()).is_err() || bad == r#"{"env":"str"}"# && false,
                "off accepted {bad:?}"
            );
            assert_eq!(fs::read_to_string(t.file()).unwrap(), bad);
        }
        fs::write(t.file(), [0xff, 0xfe, 0x00]).unwrap();
        assert!(do_on(&t.file(), &opts()).is_err());
        assert_eq!(fs::read(t.file()).unwrap(), [0xff, 0xfe, 0x00]);
    }

    #[test]
    fn settings_byte_order_mark_is_stripped_before_parsing() {
        let t = Tmp::new("bom");
        fs::write(t.file(), "\u{feff}{\"keep\":true}").unwrap();
        do_on(&t.file(), &opts()).unwrap();
        let text = fs::read_to_string(t.file()).unwrap();
        assert!(!text.starts_with('\u{feff}'));
        assert_eq!(read(&t.file()).get("keep"), Some(&Value::Bool(true)));
    }

    #[test]
    fn settings_write_is_atomic_and_leaves_no_temp_files() {
        let t = Tmp::new("atomic");
        fs::write(t.file(), r#"{"a":1}"#).unwrap();
        do_on(&t.file(), &opts()).unwrap();
        do_off(&t.file()).unwrap();
        let names: Vec<String> = fs::read_dir(&t.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["settings.json"]);

        // A failed write (target is a directory) leaves the original and no temp file behind.
        let dir_target = t.0.join("dir.json");
        fs::create_dir(&dir_target).unwrap();
        assert!(write_atomic(&dir_target, &Value::Object(vec![]), true).is_err());
        assert!(dir_target.is_dir());
        let leftovers = fs::read_dir(&t.0)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn settings_path_override_and_default() {
        assert_eq!(
            settings_path(Some("/x/s.json"), Some("/h")).unwrap(),
            PathBuf::from("/x/s.json")
        );
        assert_eq!(
            settings_path(None, Some("/h")).unwrap(),
            Path::new("/h").join(".claude").join("settings.json")
        );
        assert_eq!(
            settings_path(Some(""), Some("/h")).unwrap(),
            Path::new("/h").join(".claude").join("settings.json")
        );
        assert!(settings_path(None, None).is_err());
    }

    #[test]
    fn settings_on_refuses_to_replace_foreign_values_without_force() {
        let t = Tmp::new("foreign");
        for body in [
            r#"{"env":{"HTTPS_PROXY":"http://corp:3128"}}"#,
            r#"{"env":{"NO_PROXY":"mine.example"}}"#,
        ] {
            fs::write(t.file(), body).unwrap();
            let err = do_on(&t.file(), &opts()).unwrap_err();
            assert!(err.contains("--force"), "{err}");
            assert_eq!(fs::read_to_string(t.file()).unwrap(), body);
        }
        let force = Options {
            force: true,
            ..opts()
        };
        do_on(&t.file(), &force).unwrap();
        assert!(do_status(&t.file()).unwrap().starts_with("on:"));
        assert!(parse_options(&s(&["--force"]), 1).unwrap().force);
    }

    #[test]
    fn settings_on_is_idempotent_with_identical_values() {
        let t = Tmp::new("idem");
        do_on(&t.file(), &opts()).unwrap();
        let before = fs::read(t.file()).unwrap();
        do_on(&t.file(), &opts()).unwrap();
        assert_eq!(fs::read(t.file()).unwrap(), before);
    }

    #[test]
    fn settings_off_leaves_values_set_by_something_else() {
        let t = Tmp::new("offforeign");
        let body = r#"{"env":{"HTTPS_PROXY":"http://corp:3128","NO_PROXY":"mine"}}"#;
        fs::write(t.file(), body).unwrap();
        let msg = do_off(&t.file()).unwrap();
        assert!(msg.contains("something else"), "{msg}");
        assert_eq!(fs::read_to_string(t.file()).unwrap(), body);
        // Our HTTPS_PROXY removes both; a lone NO_PROXY is left.
        fs::write(t.file(), r#"{"env":{"NO_PROXY":"mine"}}"#).unwrap();
        do_off(&t.file()).unwrap();
        assert!(read(&t.file())
            .get("env")
            .unwrap()
            .get("NO_PROXY")
            .is_some());
    }

    #[test]
    fn settings_status_reports_foreign_proxy() {
        let t = Tmp::new("statusforeign");
        fs::write(t.file(), r#"{"env":{"HTTPS_PROXY":"http://corp:3128"}}"#).unwrap();
        assert!(do_status(&t.file())
            .unwrap()
            .starts_with("set by something else"));
        do_on(
            &t.file(),
            &Options {
                force: true,
                ..opts()
            },
        )
        .unwrap();
        assert!(do_status(&t.file()).unwrap().starts_with("on:"));
    }

    #[cfg(unix)]
    #[test]
    fn settings_dangling_symlink_is_kept_and_target_created() {
        let t = Tmp::new("dangling");
        let real = t.0.join("real.json");
        std::os::unix::fs::symlink("real.json", t.file()).unwrap();
        do_on(&t.file(), &opts()).unwrap();
        assert!(fs::symlink_metadata(t.file())
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(real.is_file());
        assert!(read(&real).get("env").is_some());
        do_off(&t.file()).unwrap();
        assert!(fs::symlink_metadata(t.file())
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn settings_write_keeps_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let t = Tmp::new("perm");
        fs::write(t.file(), "{}").unwrap();
        fs::set_permissions(t.file(), fs::Permissions::from_mode(0o600)).unwrap();
        do_on(&t.file(), &opts()).unwrap();
        let mode = fs::metadata(t.file()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
