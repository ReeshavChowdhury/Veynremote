# Upstream attribution

Veynremote is based on [Veyndesk](https://github.com/Veyndesk/veyndesk), an
open-source remote desktop application by the Veyndesk contributors.

- Original project: https://github.com/Veyndesk/veyndesk
- Baseline commit: `bfd6ec123031d9b8cef2d98d4228b9faf588b7ac`
- Original license: GNU AGPL v3.0; the original `LICENSE` is retained unchanged.
- Modified project: https://github.com/ReeshavChowdhury/Veynremote
- Modifications prepared: 2026-10-24.

This version adds an opt-in Ubuntu/X11 physical-display blanking feature while
preserving full-color remote capture, plus a separate restoration watchdog,
host configuration and desktop settings, dependency declarations, diagnostics,
tests, and documentation. It additionally adds authenticator-based two-factor
authentication, one-time recovery codes, and Linux locking after the final
viewer disconnects. The original application's runtime name and
configuration paths remain Veyndesk for compatibility.

Upstream commit history and existing source notices are preserved. This project
is independently modified; it does not claim upstream endorsement or entitlement
to the upstream project's signing services.
