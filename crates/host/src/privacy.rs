//! Host-owned, opt-in X11 screen blanking. A separate watchdog restores gamma
//! ramps when this process exits or stops sending heartbeats.
use crate::{config::HostConfig, UiEvent};
use std::{collections::HashSet, sync::{Arc, Mutex}, time::Duration};
use tokio::sync::mpsc::UnboundedSender;

pub fn spawn(cfg: Arc<Mutex<HostConfig>>, active: Arc<Mutex<HashSet<u32>>>, ui: UnboundedSender<UiEvent>) {
    std::thread::Builder::new().name("veyndesk-privacy".into()).spawn(move || {
        #[cfg(target_os = "linux")]
        {
            use std::{io::Write, process::{Child, Command, Stdio}};
            let mut helper: Option<Child> = None;
            let mut failed = false;
            loop {
                if ui.is_closed() { break; }
                let enabled = cfg.lock().unwrap().blank_screen_on_connect;
                let wanted = enabled && !active.lock().unwrap().is_empty();
                if !wanted {
                    if let Some(mut child) = helper.take() {
                        drop(child.stdin.take()); // EOF restores before the helper exits.
                        let _ = child.wait();
                    }
                    failed = false;
                } else if !failed {
                    if helper.is_none() {
                        match Command::new("python3").args(["-I", "-u", "-c", include_str!("privacy_helper.py")])
                            .stdin(Stdio::piped()).stdout(Stdio::null()).spawn() {
                            Ok(child) => helper = Some(child),
                            Err(e) => {
                                failed = true;
                                let _ = ui.send(UiEvent::Error { message: format!("Screen blanking unavailable: {e}") });
                            }
                        }
                    }
                    if let Some(child) = helper.as_mut() {
                        let exited = !matches!(child.try_wait(), Ok(None));
                        let heartbeat_failed = child.stdin.as_mut().map(|s| s.write_all(b".\n").is_err()).unwrap_or(true);
                        if exited || heartbeat_failed {
                            // Close the pipe so a still-running watchdog restores the screen.
                            drop(child.stdin.take());
                            let _ = child.wait();
                            helper = None;
                            failed = true;
                            let _ = ui.send(UiEvent::Error { message: "Screen blanking failed. Your physical screen may be visible. Use Ubuntu on Xorg with Python 3 and RandR gamma support; see the application log.".into() });
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            if let Some(mut child) = helper {
                drop(child.stdin.take());
                let _ = child.wait();
            }
        }
        #[cfg(not(target_os = "linux"))]
        { let _ = (cfg, active, ui); }
    }).expect("spawn privacy watchdog");
}
