//! Probe (K-2 review, TH-4): uninstall must not delete a LaunchAgent plist this tool did not write.
//! Runs in a throwaway HOME; the plist there is not loaded, so launchctl unload changes nothing real.
use std::process::Command;

#[test]
fn k2_probe_uninstall_keeps_foreign_plist() {
    let home = std::env::temp_dir().join(format!("k2-probe-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let agents = home.join("Library/LaunchAgents");
    std::fs::create_dir_all(&agents).unwrap();
    let foreign = agents.join("com.example.other.plist");
    let body = "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>com.example.other</string><key>ProgramArguments</key><array><string>/usr/bin/true</string></array></dict></plist>\n";
    std::fs::write(&foreign, body).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_keepalive-proxy"))
        .args(["service", "uninstall", "--label", "com.example.other"])
        .env("HOME", &home)
        .env_remove("KEEPALIVE_PROXY_HOME")
        .output()
        .unwrap();
    let still = foreign.exists();
    let _ = std::fs::remove_dir_all(&home);
    assert!(out.status.success(), "exit code not 0");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("not written by keepalive-proxy"),
        "no refusal message; stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        still,
        "foreign plist deleted; stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
