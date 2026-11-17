//! Report: reads the closed-tunnel log lines written by `logline` and prints a summary.
//! Contract (H-2): `args` holds only the arguments after the `report` subcommand.

use std::path::PathBuf;

/// One parsed closed-tunnel line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    pub target: String,
    pub up: u64,
    pub down: u64,
    pub maxgap_s: u64,
    pub end: String,
    pub deadpath: bool,
}

/// Totals over all closed tunnels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub tunnels: usize,
    pub deadpaths: usize,
    pub longest_silence_s: u64,
}

fn field<'a>(tok: &[&'a str], key: &str) -> Option<&'a str> {
    tok.iter().find_map(|t| t.strip_prefix(key))
}

/// Parses one line as written by `logline::format`; other lines give `None`.
pub fn parse_line(line: &str) -> Option<Closed> {
    let tok: Vec<&str> = line.split_whitespace().collect();
    let i = tok.iter().position(|t| *t == "close")?;
    let deadpath = tok[..i].contains(&"DEADPATH");
    let target = tok.get(i + 1)?.to_string();
    let up = field(&tok, "up=")?.parse().ok()?;
    let down = field(&tok, "down=")?.parse().ok()?;
    let maxgap_s = field(&tok, "maxgap=")?.strip_suffix('s')?.parse().ok()?;
    let end = field(&tok, "end=")?.to_string();
    Some(Closed {
        target,
        up,
        down,
        maxgap_s,
        end,
        deadpath,
    })
}

pub fn parse_log(text: &str) -> Vec<Closed> {
    text.lines().filter_map(parse_line).collect()
}

pub fn summarise(items: &[Closed]) -> Summary {
    Summary {
        tunnels: items.len(),
        deadpaths: items.iter().filter(|c| c.deadpath).count(),
        longest_silence_s: items.iter().map(|c| c.maxgap_s).max().unwrap_or(0),
    }
}

pub fn render(s: &Summary) -> String {
    format!(
        "tunnels: {}\nDEADPATH events: {}\nlongest silence survived: {}s",
        s.tunnels, s.deadpaths, s.longest_silence_s
    )
}

fn default_log_path() -> PathBuf {
    let base = match std::env::var_os("KEEPALIVE_PROXY_HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => {
            let home = std::env::var_os("HOME").unwrap_or_default();
            PathBuf::from(home).join(".keepalive-proxy")
        }
    };
    base.join("proxy.log")
}

pub fn run(args: &[String]) -> i32 {
    let path = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(default_log_path);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No log found at {}; nothing to report yet.", path.display());
            return 0;
        }
        Err(e) => {
            eprintln!("report: cannot read {}: {e}", path.display());
            return 1;
        }
    };
    let items = parse_log(&text);
    if items.is_empty() {
        println!("The log at {} has no closed tunnels yet.", path.display());
        return 0;
    }
    println!("{}", render(&summarise(&items)));
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logline;

    #[test]
    fn report_parses_plain_and_deadpath_lines() {
        let a = logline::format("t", "example.test:443", 9, 10, 20, 4, "c2u:eof@9s");
        let b = logline::format("t", "h.test:80", 50, 1, 2, 40, "u2c:err60@45s");
        let p = parse_line(&a).unwrap();
        assert_eq!(p.target, "example.test:443");
        assert_eq!((p.up, p.down, p.maxgap_s), (10, 20, 4));
        assert_eq!(p.end, "c2u:eof@9s");
        assert!(!p.deadpath);
        assert!(parse_line(&b).unwrap().deadpath);
    }

    #[test]
    fn report_ignores_other_lines() {
        assert!(parse_line("listening on 127.0.0.1:1").is_none());
        assert!(parse_line("").is_none());
        assert!(parse_line("t close h up=x down=1 maxgap=1s end=e").is_none());
    }

    #[test]
    fn report_summarises_counts_and_longest_silence() {
        let text = format!(
            "noise\n{}\n{}\n{}\n",
            logline::format("t", "a:1", 1, 0, 0, 7, "c2u:eof@1s"),
            logline::format("t", "b:1", 1, 0, 0, 90, "u2c:err60@1s"),
            logline::format("t", "c:1", 1, 0, 0, 30, "c2u:werr60"),
        );
        let s = summarise(&parse_log(&text));
        assert_eq!(
            s,
            Summary {
                tunnels: 3,
                deadpaths: 2,
                longest_silence_s: 90
            }
        );
        let out = render(&s);
        assert!(out.contains("tunnels: 3") && out.contains("DEADPATH events: 2"));
        assert!(out.contains("90s"));
    }

    #[test]
    fn report_empty_summary_is_zero() {
        assert_eq!(summarise(&[]), Summary::default());
    }

    #[test]
    fn report_missing_and_empty_log_exit_zero() {
        let dir = std::env::temp_dir().join(format!("kp-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("none.log");
        assert_eq!(run(&[missing.display().to_string()]), 0);
        let empty = dir.join("empty.log");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(run(&[empty.display().to_string()]), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
