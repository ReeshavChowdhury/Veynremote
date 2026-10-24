# Ubuntu screen blanking (experimental)

Use the Windows app as usual. On the **Ubuntu host**, open Settings and enable
**Blank this computer’s displays during remote sessions (experimental)**.
The setting is saved locally and defaults to off. The Windows viewer needs no
protocol changes, so existing viewers can connect.

Requirements:

- An X11 desktop session: choose **Ubuntu on Xorg** at login. Wayland is unsupported.
- Python 3, libX11, and libXrandr (`sudo apt install python3 libx11-6 libxrandr2`).
- A display driver that applies RandR gamma ramps to the physical displays.

The host checks for authenticated sessions once a second. While any viewer is
connected, a separate helper saves each CRTC's exact gamma ramps and sets them to
zero. Capture continues reading the desktop framebuffer. After the last viewer
disconnects, or when you disable the setting, the helper restores the saved ramps.
A closed heartbeat pipe or five seconds without a heartbeat also restores them,
including when the host process crashes. Failed helper startup is reported in the
host UI; detailed errors go to stderr. Toggle the setting off and on to retry.

## Limits and verification

This is experimental display dimming, not a guaranteed privacy boundary. Gamma
support varies by driver. A hardware cursor may remain visible. Monitor hotplug,
color-management software, display reconfiguration, and killing the helper itself
can defeat blanking or restoration. The initial session may be visible for about
one second before the helper starts. Local keyboard and mouse input remains enabled.
The monitor backlight may stay on even when desktop pixels are black.

Before relying on this feature, have someone check the physical Ubuntu displays
while you connect from Windows. Confirm that the Windows viewer still receives a
normal desktop, test all monitors, then disconnect and confirm restoration. Also
test multiple viewers (disconnecting one must not restore while another remains),
disabling the setting during a session, and terminating the host process. Do not
change monitor layout during the test. If restoration fails, logging out of Xorg
or restarting the display session resets the display configuration.

## Development checks

Run `python3 -m unittest discover -s crates/host/tests -v` for helper restoration
tests, and `cargo test --workspace` for Rust checks. Build the desktop app using
the README instructions. Live X11 tests now confirm that active display gamma ramps become zero and are
restored exactly after disconnect and heartbeat timeout. Run the opt-in native
tests with `VEYNDESK_TEST_X11_BLANKING=1 python3 -m unittest discover -s crates/host/tests -v`;
these briefly blank the current display. Physical-monitor observation and a
Windows-to-Ubuntu session test remain necessary for each new setup. The desktop release build and all workspace tests passed. The live
`privacy_capture_check` example confirmed that the actual Xcap source continues
receiving a colored 1920×1080 desktop while blanking is active. The repository
owner also confirmed that the Windows-to-Ubuntu setup worked. These checks cover
the tested setup; verify physical monitor behavior on each new system.
