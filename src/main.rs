//! keepalive-proxy: HTTP CONNECT proxy with aggressive TCP keepalive, plus its commands.
//! No dependencies.

mod cli;
mod json;
mod logline;
mod report;
mod service;
mod settings;
mod sock;
mod tunnel;

use cli::Command;

/// Dispatch: `settings::run` gets the command word (on/off/status) first, then the rest;
/// `report::run` and `service::run` get only the arguments after the subcommand.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match cli::parse(&args) {
        Ok((Command::Serve, _)) => tunnel::serve(),
        Ok((Command::On, rest)) | Ok((Command::Off, rest)) | Ok((Command::Status, rest)) => {
            let mut full = vec![cli::name(&args).to_string()];
            full.extend(rest);
            settings::run(&full)
        }
        Ok((Command::Report, rest)) => report::run(&rest),
        Ok((Command::Service, rest)) => service::run(&rest),
        Err(msg) => {
            eprintln!("{msg}");
            2
        }
    };
    std::process::exit(code);
}
