//! Linux desktop locking after the final authenticated viewer leaves.
use crate::UiEvent;
use std::collections::HashSet;
use tokio::sync::mpsc::UnboundedSender;

pub(crate) fn remove_last(active: &mut HashSet<u32>, sid: u32) -> bool {
    active.remove(&sid) && active.is_empty()
}

fn request_lock(mut run: impl FnMut(&str, &[&str]) -> bool) -> bool {
    // The session bus addresses this user's GNOME desktop, not other users.
    run("gdbus", &["call", "--session", "--dest", "org.gnome.ScreenSaver",
        "--object-path", "/org/gnome/ScreenSaver", "--method", "org.gnome.ScreenSaver.Lock", "--timeout", "5"])
        || run("loginctl", &["lock-session", "auto"])
}

pub(crate) fn spawn(ui: UnboundedSender<UiEvent>) {
    std::thread::spawn(move || {
        #[cfg(target_os = "linux")]
        let locked = request_lock(|program, args| {
            std::process::Command::new(program).args(args)
                .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
        });
        #[cfg(not(target_os = "linux"))]
        let locked = false;
        if !locked {
            let _ = ui.send(UiEvent::Error { message: "Could not lock this desktop after disconnect. Check that the desktop supports screen locking.".into() });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_final_real_session_triggers_locking() {
        let mut sessions = HashSet::from([1, 2]);
        assert!(!remove_last(&mut sessions, 99));
        assert!(!remove_last(&mut sessions, 1));
        assert!(remove_last(&mut sessions, 2));
        assert!(!remove_last(&mut sessions, 2));
    }
    #[test]
    fn tries_fallback_and_reports_failure() {
        let mut attempted = vec![];
        assert!(request_lock(|program, _| { attempted.push(program.to_string()); program == "loginctl" }));
        assert_eq!(attempted, ["gdbus", "loginctl"]);
        assert!(!request_lock(|_, _| false));
        let mut calls = 0;
        assert!(request_lock(|_, _| { calls += 1; true }));
        assert_eq!(calls, 1);
    }
}
