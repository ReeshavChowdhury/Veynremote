# Authenticator and disconnect locking

## Set up Google Authenticator

1. Install the updated Ubuntu host. Use the updated Windows viewer, or open the
   updated Ubuntu host's browser viewer from Windows.
2. On Ubuntu, open **Settings → Two-factor authentication → Set up authenticator**.
3. Scan the QR code in Google Authenticator (or another TOTP-compatible app). A
   manual setup key is also displayed. Keep this key private.
4. Enter the six-digit phone code and choose **Verify and enable**. Setup expires
   after ten minutes. Authentication is not enabled until verification succeeds.
5. Save the ten recovery codes offline. They are displayed only at enrollment;
   each can be used once in place of a phone code.
6. Wait for the next phone code, then connect from Windows using the normal
   password plus the **Authenticator or recovery code** field.

The code used to complete setup is already consumed. Use a new code for the first
connection, for each additional viewer, and to disable two-factor authentication.
Enabling two-factor authentication reconnects the host and ends existing sessions
so previously issued password-only challenges cannot grant access. If auto-lock
is enabled, ending the last such session also locks the desktop.

Two-factor authentication applies to **all new sessions**, including unattended
password sessions and sessions that also require local approval. It defaults to
off. Older viewers cannot authenticate once it is enabled; they cannot bypass it.
Update any separately deployed relay/server too, because it forwards the new
challenge and proof fields. P2P sessions use the host's updated built-in server.

Google Authenticator can generate codes without an internet connection. Veynremote
validates them on the host; it does not send the setup key to Google. This uses
[RFC 6238 TOTP](https://www.rfc-editor.org/rfc/rfc6238), with HMAC-SHA1, six digits,
and a 30-second period. See [Google's Authenticator help](https://support.google.com/accounts/answer/1066447).
Keep the phone and Ubuntu clocks accurate. A one-step clock difference is allowed.

## Recovery and disabling

On Ubuntu, enter an unused authenticator or recovery code in Settings and choose
**Disable two-factor authentication**. A used code is rejected. To create a new
set of recovery codes, disable and enroll again, then save the new codes.

Keep at least one recovery code outside the remote computer. If the phone and
all recovery codes are lost, recovery requires trusted local access to the
Ubuntu host's configuration; there is no remote bypass.

The shared TOTP setup secret is stored locally in the host configuration, not
hashed (verification needs the secret). On Unix, updated settings are written
atomically with owner-only `0600` permissions. Recovery codes are stored only as
SHA-256 hashes. Backups of the host configuration also contain sensitive data.
The setup key, phone codes, and recovery codes must not be committed to Git.

## Lock after disconnect

On the Ubuntu host, enable **Settings → Lock this desktop after the last viewer
disconnects**. This option is separate from screen blanking and defaults to off.

Only the transition from one or more authenticated viewers to none triggers a
lock request. Wrong-password attempts and one viewer leaving while another
remains do not lock the desktop. A detected network/server disconnection and the
host's Disconnect All action also end sessions and trigger locking as appropriate.

Linux hosts first request locking through GNOME's session D-Bus service, then
try `loginctl lock-session auto` if that fails. The desktop must support locking;
command failure appears in the host UI. Auto-lock does not unlock the desktop
for later remote connections. A hard host/process crash is not guaranteed to
trigger a lock request; it is separate from the screen-blanking watchdog.

## Authentication protocol notes

The viewer sends a second-factor proof bound to the password-derived secret and
fresh host nonce, rather than transmitting the six-digit code directly:

```text
code_hash = SHA256(normalized authenticator or recovery code)
second_factor_proof = SHA256(password_secret + ":" + nonce + ":second_factor:" + code_hash)
```

The host validates the password first, checks the second factor, and persists
its consumed time step or recovery-code removal **before** granting access. A
save failure denies authentication. The persisted time step prevents replay
across host restarts. Eight failed remote authentication attempts in five minutes
block further attempts during that interval. Local confirmation/disable attempts
are limited to eight in five minutes as well.

This change does not add end-to-end encryption to relay sessions or TLS to direct
LAN WebSockets. It retains the upstream transport security described in the README.

## Checks

`cargo test --workspace --locked` covers RFC vectors, enrollment QR generation,
clock skew, replay, recovery-code consumption, compatibility, and an isolated
real host/server/viewer handshake. The integration test uses a test-pattern
screen, temporary settings, and disabled locking/blanking so it cannot lock or
blank a user's desktop. Linux lock selection and last-viewer behavior are tested
with a mock command runner. It does not issue a real lock during tests.

`node --test web/tests/auth.test.cjs` checks the actual viewer authentication
code, recovery-code normalization, missing codes, and password-only compatibility.
