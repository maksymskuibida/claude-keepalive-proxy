//! The closed-tunnel log line; its fields are the H-2 contract read by report.
//! This module is finished, not a stub.

/// errno 60 (ETIMEDOUT) on the upstream leg means keepalive probes went unanswered: the path was dead.
pub fn is_dead_path(end: &str) -> bool {
    end.starts_with("u2c:err60") || end.starts_with("c2u:werr60")
}

/// Formats the line written when a tunnel closes. Dead paths carry the DEADPATH marker.
pub fn format(
    stamp: &str,
    target: &str,
    secs: u64,
    up: u64,
    down: u64,
    maxgap_s: u64,
    end: &str,
) -> String {
    format!(
        "{stamp} {} {target} after {secs}s up={up} down={down} maxgap={maxgap_s}s end={end}",
        if is_dead_path(end) {
            "DEADPATH close"
        } else {
            "close"
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logline_marks_deadpath_for_errno_60() {
        let l = format("01:02:03Z", "h:443", 50, 1, 2, 40, "u2c:err60@45s");
        assert_eq!(
            l,
            "01:02:03Z DEADPATH close h:443 after 50s up=1 down=2 maxgap=40s end=u2c:err60@45s"
        );
        assert!(format("t", "h", 1, 0, 0, 0, "c2u:werr60").contains("DEADPATH"));
    }

    #[test]
    fn logline_plain_close_has_no_marker() {
        let l = format("t", "h:443", 1, 0, 0, 0, "c2u:eof@1s");
        assert!(!l.contains("DEADPATH"));
        assert!(l.contains(" close h:443 after 1s"));
        assert!(!is_dead_path("u2c:err54@3s"));
    }
}
