//! Live X11 check: briefly blanks physical displays and checks that the actual
//! Veyndesk capture source still receives colored desktop pixels.
//! Run intentionally: cargo run -p veyndesk-host --example privacy_capture_check
use std::{io::Write, process::{Command, Stdio}, time::Duration};
use veyndesk_host::video::{Frame, FrameSource, XcapSource};

fn counts(frame: &Frame) -> (usize, usize) {
    let mut lit = 0;
    let mut colored = 0;
    for p in frame.rgba.chunks_exact(4) {
        let hi = p[0].max(p[1]).max(p[2]);
        let lo = p[0].min(p[1]).min(p[2]);
        lit += usize::from(hi > 20);
        colored += usize::from(hi.saturating_sub(lo) > 15);
    }
    (lit, colored)
}

fn main() -> anyhow::Result<()> {
    let mut source = XcapSource::new();
    let before = source.capture(0)?;
    let baseline = counts(&before);
    anyhow::ensure!(baseline.0 > 1000 && baseline.1 > 1000,
        "Show a colorful desktop before running this check");
    let mut child = Command::new("python3")
        .args(["-I", "-u", "-c", include_str!("../src/privacy_helper.py")])
        .stdin(Stdio::piped()).spawn()?;
    // Dropping the pipe on any error allows the watchdog to restore the display.
    let result = (|| -> anyhow::Result<()> {
        child.stdin.as_mut().unwrap().write_all(b".\n")?;
        std::thread::sleep(Duration::from_secs(1));
        anyhow::ensure!(child.try_wait()?.is_none(), "blanking helper exited early");
        let during = source.capture(0)?;
        let actual = counts(&during);
        anyhow::ensure!((before.width, before.height) == (during.width, during.height),
            "capture dimensions changed");
        anyhow::ensure!(actual.0 > baseline.0 / 2 && actual.1 > baseline.1 / 2,
            "desktop capture lost brightness or color while blanking");
        println!("PASS: {}x{} desktop capture keeps color during display blanking", during.width, during.height);
        Ok(())
    })();
    drop(child.stdin.take());
    let status = child.wait()?;
    result?;
    anyhow::ensure!(status.success(), "blanking helper failed");
    Ok(())
}
