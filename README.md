# Veynremote

Remote desktop with **Ubuntu screen blanking**: control your Ubuntu computer from
Windows in full color while Ubuntu’s physical monitor shows a black screen.

Veynremote is a derivative of **[Veyndesk](https://github.com/Veyndesk/veyndesk)**.
The original remote desktop application, Rust host, WebRTC connectivity, server,
and viewer were created by the Veyndesk contributors. This repository adds an
opt-in Linux/X11 screen blanking feature, its settings UI, watchdog, and tests.
See [upstream attribution](UPSTREAM.md). The original [AGPL-3.0 license](LICENSE)
is retained.

## What this version adds

- **Black screen on Ubuntu, normal desktop on Windows.** Display gamma ramps
  are blanked on the Ubuntu host while desktop capture continues in color.
- **Automatic session handling.** Blanking starts when an authenticated viewer
  connects and ends after the last viewer disconnects.
- **Automatic restoration.** Original display gamma ramps are saved and restored.
  A separate watchdog restores them if the host exits or stops sending heartbeats
  for five seconds.
- **Local control.** Enable or disable the feature in the Ubuntu host’s Settings.
  It is off by default in a fresh installation.
- **Existing viewer compatibility.** The screen blanking feature needs an updated
  Ubuntu host; it does not change the viewer protocol.

The Windows-to-Ubuntu setup has been tested by this repository’s owner, who
confirmed that the physical screen blanking worked. Automated live X11 checks
also verified gamma restoration and full-color desktop capture. Other monitor
and graphics-driver combinations still need testing.

## Use from Windows to Ubuntu

1. On Ubuntu, use an **X11 session**. If necessary, select **Ubuntu on Xorg** at
   the login screen.
2. Build and start this version of the desktop app on Ubuntu.
3. On Ubuntu, open **Settings** and enable **Blank this computer’s displays during
   remote sessions (experimental)**.
4. Open Veyndesk on Windows and connect using the Ubuntu app’s ID and password.
5. Accept the connection on Ubuntu, unless you configured unattended access.
6. Windows should show the normal Ubuntu desktop while the Ubuntu monitor is
   black. Disconnect to restore the physical display.

For unattended access, set a permanent password in the Ubuntu app’s Settings.
Keep the host app running while connecting.

**Scope:** screen blanking currently requires Linux/X11, Python 3, libX11,
libXrandr, and a graphics driver with working RandR gamma support. Wayland,
Windows-host blanking, and macOS-host blanking are unsupported.

**Limitations:** blanking can take about one second to start. The physical
monitor’s backlight or cursor may remain visible. Local input stays enabled.
Monitor hotplug, color-management software, and display reconfiguration can
interfere with blanking. Verify every physical display before relying on it for
privacy. This is experimental display blanking, not a guaranteed privacy boundary.
See [setup, behavior, and troubleshooting](docs/screen-blanking.md).

## Build on Ubuntu 24.04

Install a current stable [Rust toolchain](https://rustup.rs/) and these packages:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libwebkit2gtk-4.1-dev \
  libssl-dev libxdo-dev libxcb1-dev libxrandr-dev libdbus-1-dev \
  libayatana-appindicator3-dev librsvg2-dev libgbm-dev libpipewire-0.3-dev \
  libwayland-dev libegl-dev clang libclang-dev file patchelf python3
```

Clone and build:

```bash
git clone https://github.com/ReeshavChowdhury/Veynremote.git
cd Veynremote
cargo test --workspace --locked
cargo build --manifest-path desktop/src-tauri/Cargo.toml --release --locked
./desktop/src-tauri/target/release/veyndesk-desktop
```

The executable and application UI retain the upstream **Veyndesk** name. The
repository name is **Veynremote**. Existing IDs, passwords, and configuration
remain in the upstream application’s configuration directory.

To build installers, install Node.js/npm and run:

```bash
cd desktop
npx --yes @tauri-apps/cli@2 build
```

Do not run both the original and modified host apps simultaneously: they share
configuration and the built-in server port.

## Tests

```bash
cargo test --workspace --locked
python3 -m unittest discover -s crates/host/tests -v
```

The following optional checks **temporarily blank the current X11 displays**.
Run them intentionally from an X11 desktop:

```bash
VEYNDESK_TEST_X11_BLANKING=1 python3 -m unittest discover -s crates/host/tests -v
cargo run --locked -p veyndesk-host --example privacy_capture_check
```

The native tests check exact restoration after disconnect and heartbeat timeout.
The capture check verifies that the actual desktop capture source keeps receiving
colored pixels while physical display blanking is active.

## Features inherited from Veyndesk

- Direct WebRTC connections, with automatic relay fallback.
- ID-and-password connections and optional unattended access.
- Adaptive screen sharing, multiple monitors, input, and clipboard sync.
- Optional self-hosted server, browser viewer, and administration pages.

Direct WebRTC sessions use DTLS encryption. The upstream relay uses HTTPS through
Cloudflare and is not end-to-end encrypted. Direct LAN WebSocket connections are
not encrypted by default. Screen blanking does not change transport security.

The app inherits Veyndesk’s use of ntfy.sh for discovery, Google/Cloudflare STUN
for direct connectivity, and optional Cloudflare tunnels for relay access.
See [upstream documentation](https://veyndesk.github.io/) for the original
architecture and service details.

## Project layout

| Path | Purpose |
| --- | --- |
| `crates/host` | Desktop capture, input, connections, and screen blanking |
| `crates/protocol` | Shared messages and authentication |
| `crates/server` | Built-in and standalone server |
| `desktop` | Tauri desktop app and settings |
| `web` | Browser viewer and administration pages |
| `docs/screen-blanking.md` | Screen blanking setup and limitations |

## Attribution and license

Based on [Veyndesk](https://github.com/Veyndesk/veyndesk), maintained upstream by
its contributors, including [KavindaBanukaVC](https://github.com/KavindaBanukaVC).
Veyndesk also acknowledges inspiration from
[RustDesk](https://github.com/rustdesk/rustdesk).

Veynremote is an independently modified version and is not an official Veyndesk
release. Upstream signing certificates and release approvals do not apply to
builds from this repository.

Licensed under [GNU AGPL v3.0](LICENSE). Preserve the license and upstream notices
when redistributing this project.
