# Veynremote — Remote desktop with Ubuntu screen blanking and 2FA

An open-source remote desktop project focused on private Windows-to-Ubuntu access.

With **Ubuntu screen blanking**, control your Ubuntu computer from
Windows in full color while Ubuntu’s physical monitor shows a black screen.
Add **Google Authenticator two-factor authentication** and **auto-lock after
disconnect** from the host’s Settings.

Veynremote is a derivative of **[Veyndesk](https://github.com/Veyndesk/veyndesk)**.
The original remote desktop application, Rust host, WebRTC connectivity, server,
and viewer were created by the Veyndesk contributors. This repository adds
Linux/X11 screen blanking, authenticator-based two-factor authentication,
Linux auto-lock after disconnect, settings, and tests.
See [upstream attribution](UPSTREAM.md). The original [AGPL-3.0 license](LICENSE)
is retained.

**[Project website and setup guide](https://reeshavchowdhury.github.io/veynremote/)** · [Reeshav’s project portfolio](https://reeshavchowdhury.github.io/)

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
- **Existing viewer compatibility for blanking.** Screen blanking alone works with
  existing viewers. Authenticator support requires an updated viewer/server.

The Windows-to-Ubuntu setup has been tested by this repository’s owner, who
confirmed that the physical screen blanking worked. Automated live X11 checks
also verified gamma restoration and full-color desktop capture. Other monitor
and graphics-driver combinations still need testing.

## Authenticator and auto-lock

**Google Authenticator:** on the host, open Settings → Two-factor authentication,
scan the setup QR code, verify a phone code, and save the recovery codes. New
connections then require both the remote password and an unused authenticator
or recovery code, including unattended access. Use an updated viewer and server;
older viewers cannot connect when two-factor authentication is enabled.

**Auto-lock:** on Ubuntu, enable **Lock this desktop after the last viewer
disconnects** in Settings. It requests the desktop lock after the final
authenticated viewer leaves and works independently of screen blanking.

Both options default to off in a fresh installation. Read the
[setup and recovery instructions](docs/security-settings.md) before enabling them.

## Updating and Windows compatibility

Install the updated host on Ubuntu to get the new Settings controls. Building
new source does not update an already running executable: replace the executable
and restart the app after disconnecting active viewers. Existing IDs and passwords
are retained in the application’s configuration directory.

| Feature | Updated Ubuntu host | Updated Windows viewer |
| --- | --- | --- |
| Ubuntu physical-screen blanking | Required | Not required |
| Auto-lock after the last viewer disconnects | Required | Not required |
| Authenticator or recovery-code login | Required | Required, or use the updated browser viewer |

For browser access from Windows on the same trusted network, open
`http://<ubuntu-ip>:21118/` while the Ubuntu app’s built-in server is running.
Enter the Ubuntu app’s ID, password, and authenticator/recovery code. Replace
`<ubuntu-ip>` with the Ubuntu computer’s LAN address. This direct HTTP/LAN path
is not encrypted; use an appropriate secure connection for untrusted networks.

Screen blanking and auto-lock do not require enrolling an authenticator.
Two-factor authentication becomes active only after scanning the QR code and
successfully verifying a phone code. Save the ten single-use recovery codes,
and wait for the next phone code before the first remote login: the setup code
has already been consumed. A separately deployed server must also be updated.

Build artifacts, when available, are attached to successful
[GitHub Actions build runs](https://github.com/ReeshavChowdhury/Veynremote/actions/workflows/build.yml).
The upstream Veyndesk downloads do not include these Veynremote changes.

## Use from Windows to Ubuntu

1. On Ubuntu, use an **X11 session**. If necessary, select **Ubuntu on Xorg** at
   the login screen.
2. Build and start this version of the desktop app on Ubuntu.
3. On Ubuntu, open **Settings** and enable **Blank this computer’s displays during
   remote sessions (experimental)**.
4. Open Veyndesk on Windows and connect using the Ubuntu app’s ID and password.
   If two-factor authentication is enabled, use an updated viewer and enter a
   fresh authenticator code or unused recovery code as well.
5. Accept the connection on Ubuntu, unless you configured unattended access.
6. Windows should show the normal Ubuntu desktop while the Ubuntu monitor is
   black. Disconnect to restore the physical display. If auto-lock is enabled,
   Ubuntu requests a desktop lock after the final viewer leaves.

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
node --test web/tests/auth.test.cjs  # requires Node.js
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

## Frequently asked questions

### Can I control Ubuntu from Windows while its physical monitor stays black?

Yes. Enable screen blanking on the Ubuntu host and connect from Windows. The
Windows viewer receives the desktop in color while supported Ubuntu physical
displays show black. This requires Ubuntu on Xorg/X11 and working RandR gamma
support; see the [screen blanking guide](docs/screen-blanking.md).

### Can I use Google Authenticator for remote desktop access?

Yes. Enroll a TOTP authenticator in the host’s Settings, verify a phone code, and
save the recovery codes. Each new remote connection then requires the password
and an unused authenticator or recovery code. Updated viewers and servers are
required; see [two-factor authentication setup](docs/security-settings.md).

### Does Ubuntu lock when I disconnect?

With auto-lock enabled, the host requests a Linux desktop lock after the last
authenticated viewer disconnects. Screen blanking and auto-lock are independent
settings. The Linux desktop must support screen locking.

## Project layout

| Path | Purpose |
| --- | --- |
| `crates/host` | Capture, input, connections, blanking, two-factor verification, and locking |
| `crates/protocol` | Shared messages and authentication |
| `crates/server` | Built-in and standalone server |
| `desktop` | Tauri desktop app and settings |
| `web` | Browser viewer and administration pages |
| `docs/screen-blanking.md` | Screen blanking setup and limitations |
| `docs/security-settings.md` | Authenticator setup, recovery codes, and auto-lock |

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
