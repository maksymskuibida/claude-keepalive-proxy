//! Command line parsing: serve (default), on, off, status, report, service.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Command {
    Serve,
    On,
    Off,
    Status,
    Report,
    Service,
}

/// Returns the command and the arguments after it. No arguments means serve.
pub fn parse(args: &[String]) -> Result<(Command, Vec<String>), String> {
    let Some(first) = args.first() else {
        return Ok((Command::Serve, Vec::new()));
    };
    let cmd = match first.as_str() {
        "serve" => Command::Serve,
        "on" => Command::On,
        "off" => Command::Off,
        "status" => Command::Status,
        "report" => Command::Report,
        "service" => Command::Service,
        other => {
            return Err(format!(
                "unknown command '{other}'; use serve, on, off, status, report or service"
            ))
        }
    };
    Ok((cmd, args[1..].to_vec()))
}

/// The command word as typed (first argument), or "serve".
pub fn name(args: &[String]) -> &str {
    args.first().map(String::as_str).unwrap_or("serve")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cli_default_is_serve() {
        assert_eq!(parse(&[]).unwrap().0, Command::Serve);
    }

    #[test]
    fn cli_parses_each_subcommand() {
        for (w, c) in [
            ("serve", Command::Serve),
            ("on", Command::On),
            ("off", Command::Off),
            ("status", Command::Status),
            ("report", Command::Report),
            ("service", Command::Service),
        ] {
            assert_eq!(parse(&v(&[w])).unwrap().0, c);
        }
    }

    #[test]
    fn cli_passes_rest_and_rejects_unknown() {
        let (c, rest) = parse(&v(&["report", "--x", "1"])).unwrap();
        assert_eq!(c, Command::Report);
        assert_eq!(rest, v(&["--x", "1"]));
        assert!(parse(&v(&["bogus"])).is_err());
    }
}
