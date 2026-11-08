// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Veynremote desktop app — one program, like AnyDesk:
//! * shows this PC's ID + password and lets others control it (accept / deny prompt),
//! * opens remote screens of other PCs,
//! * optionally runs a Veynremote server on this PC so a whole office/LAN needs nothing else.

mod discovery;
mod internet;

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use veyndesk_protocol::{ct_eq, password_secret};
use veyndesk_host::{
    config::{config_path, one_time_password, Credentials, HostConfig},
    AuthConfig, HostCommand, UiEvent,
};

/// Port used when this PC runs the embedded server.
const LAN_PORT: u16 = 21118;

struct AppState {
    allow_exit: Arc<AtomicBool>,
    cfg: Arc<Mutex<HostConfig>>,
    auth: Arc<Mutex<AuthConfig>>,
    otp: Mutex<String>,
    pending_factor: Mutex<Option<(veyndesk_host::security::TwoFactorConfig, std::time::Instant)>>,
    factor_attempts: Mutex<Vec<std::time::Instant>>,
    id: Arc<Mutex<Option<String>>>,
    status: Arc<Mutex<String>>,
    cmd: UnboundedSender<HostCommand>,
    /// `Some(error)` if starting the embedded server failed; `None` while not attempted.
    server_state: Arc<Mutex<ServerState>>,
    net: Arc<internet::Internet>,
}

#[derive(Clone, Serialize, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
enum ServerState {
    #[default]
    Off,
    Running { lan_url: String },
    Failed { error: String },
}

#[derive(Serialize)]
struct Info {
    id: Option<String>,
    password: String,
    server: String,
    name: String,
    permanent: bool,
    ask_to_accept: bool,
    blank_screen_on_connect: bool,
    screen_blanking_supported: bool,
    lock_after_disconnect: bool,
    two_factor_enabled: bool,
    recovery_codes_remaining: usize,
    start_at_login: bool,
    start_at_login_supported: bool,
    run_server: bool,
    server_state: ServerState,
    status: String,
    lan_ip: String,
    internet: bool,
    internet_state: internet::NetState,
    team_code: Option<String>,
    version: &'static str,
}

fn rebuild_auth(state: &AppState) {
    let cfg = state.cfg.lock().unwrap().clone();
    let otp = state.otp.lock().unwrap().clone();
    let mut a = AuthConfig::build(&cfg, Some(&otp));
    a.ask_on_one_time = cfg.ask_to_accept;
    *state.auth.lock().unwrap() = a;
}

fn http_base(server: &str) -> String {
    let s = server.trim().trim_end_matches('/');
    let s = s.strip_suffix("/ws/host").unwrap_or(s);
    if let Some(r) = s.strip_prefix("wss://") {
        format!("https://{r}")
    } else if let Some(r) = s.strip_prefix("ws://") {
        format!("http://{r}")
    } else if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("http://{s}")
    }
}

/// This PC's LAN address (no packets are sent; UDP connect only picks a route).
fn lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

#[allow(dead_code)]
struct InstanceLock(Option<std::fs::File>);

#[cfg(target_os = "linux")]
fn acquire_instance_lock() -> Result<Option<InstanceLock>, String> {
    use std::{fs::OpenOptions, os::fd::AsRawFd};

    // Keep a process-wide advisory lock so repeated launcher clicks cannot register
    // this host ID from competing Veynremote processes and evict each other.
    #[link(name = "c")]
    extern "C" {
        fn flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int) -> std::os::raw::c_int;
    }
    const LOCK_EX: std::os::raw::c_int = 2;
    const LOCK_NB: std::os::raw::c_int = 4;

    let path = config_path().with_file_name("instance.lock");
    let parent = path.parent().ok_or("Invalid Veynremote instance-lock path.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let file = OpenOptions::new().create(true).read(true).write(true).open(path).map_err(|e| e.to_string())?;
    let result = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if result == 0 {
        Ok(Some(InstanceLock(Some(file))))
    } else if std::io::Error::last_os_error().raw_os_error() == Some(11) {
        Ok(None)
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(not(target_os = "linux"))]
fn acquire_instance_lock() -> Result<Option<InstanceLock>, String> {
    Ok(Some(InstanceLock(None)))
}

fn autostart_file() -> Result<std::path::PathBuf, String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config")))
        .ok_or("Could not find the user configuration directory.")?;
    if !config.is_absolute() {
        return Err("The user configuration directory must be an absolute path.".into());
    }
    // Keep the existing entry path so upgrades replace the Veyndesk autostart
    // entry instead of leaving a second launcher behind.
    Ok(config.join("autostart").join("Veyndesk.desktop"))
}

fn autostart_enabled() -> bool {
    cfg!(target_os = "linux") && autostart_file().is_ok_and(|path| path.is_file())
}

fn desktop_exec_argument(argument: &str) -> String {
    argument.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%")
}

/// Starts the embedded server (idempotent). Returns the state after the attempt.
fn start_embedded_server(server_state: &Arc<Mutex<ServerState>>) -> ServerState {
    if let ServerState::Running { .. } = &*server_state.lock().unwrap() {
        return server_state.lock().unwrap().clone();
    }
    let new_state = match std::net::TcpListener::bind(("0.0.0.0", LAN_PORT)) {
        Ok(listener) => {
            let _ = listener.set_nonblocking(true);
            let data = config_path().parent().map(|p| p.join("server")).unwrap_or_else(|| "server-data".into());
            let st = server_state.clone();
            tauri::async_runtime::spawn(async move {
                let result = match tokio::net::TcpListener::from_std(listener) {
                    Ok(l) => veyndesk_server::serve_on(l, data).await,
                    Err(e) => Err(e.into()),
                };
                if let Err(e) = result {
                    *st.lock().unwrap() = ServerState::Failed { error: e.to_string() };
                }
            });
            ServerState::Running { lan_url: format!("http://{}:{LAN_PORT}", lan_ip()) }
        }
        Err(e) => ServerState::Failed { error: format!("port {LAN_PORT} is busy: {e}") },
    };
    *server_state.lock().unwrap() = new_state.clone();
    new_state
}

// ------------------------------------------------------------------ commands

#[tauri::command]
fn get_info(state: State<'_, AppState>) -> Info {
    let cfg = state.cfg.lock().unwrap();
    Info {
        id: state.id.lock().unwrap().clone().or_else(|| cfg.id.clone()),
        password: state.otp.lock().unwrap().clone(),
        server: cfg.server.clone(),
        name: cfg.name.clone(),
        permanent: cfg.permanent.is_some(),
        ask_to_accept: cfg.ask_to_accept,
        blank_screen_on_connect: cfg.blank_screen_on_connect,
        screen_blanking_supported: cfg!(target_os = "linux"),
        lock_after_disconnect: cfg.lock_after_disconnect,
        two_factor_enabled: cfg.two_factor.is_some(),
        recovery_codes_remaining: cfg.two_factor.as_ref().map_or(0, |factor| factor.recovery_hashes.len()),
        start_at_login: autostart_enabled(),
        start_at_login_supported: cfg!(target_os = "linux"),
        run_server: cfg.run_server,
        server_state: state.server_state.lock().unwrap().clone(),
        status: state.status.lock().unwrap().clone(),
        lan_ip: lan_ip(),
        internet: cfg.internet,
        internet_state: state.net.state.lock().unwrap().clone(),
        team_code: cfg.team_code.clone(),
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Server PC: make the built-in server reachable from any network (free Cloudflare tunnel).
#[tauri::command]
fn enable_internet(state: State<'_, AppState>, enable: bool) -> Result<Option<String>, String> {
    let code = {
        let mut cfg = state.cfg.lock().unwrap();
        cfg.internet = enable;
        if enable {
            cfg.run_server = true;
            if !matches!(*state.server_state.lock().unwrap(), ServerState::Running { .. }) {
                cfg.server = format!("http://127.0.0.1:{LAN_PORT}");
            }
            if cfg.team_code.is_none() {
                cfg.team_code = Some(internet::new_code());
            }
        }
        cfg.save().map_err(|e| e.to_string())?;
        cfg.team_code.clone()
    };
    if enable {
        if let ServerState::Failed { error } = start_embedded_server(&state.server_state) {
            return Err(error);
        }
        let _ = state.cmd.send(HostCommand::Reconnect);
        state.net.set_code(code.clone());
        state.net.start(LAN_PORT);
    } else {
        // Keep the tunnel itself: it is also the automatic relay for direct connections.
        state.net.set_code(None);
    }
    Ok(code)
}

/// Other PCs: join the team's server from anywhere using the team code.
#[tauri::command]
async fn join_team(state: State<'_, AppState>, code: String) -> Result<String, String> {
    let code = internet::normalize_code(&code).ok_or("A team code looks like ABCD-2345 (8 letters/digits).")?;
    let c2 = code.clone();
    let url = tauri::async_runtime::spawn_blocking(move || internet::resolve(&c2))
        .await
        .map_err(|e| e.to_string())??;
    {
        let mut cfg = state.cfg.lock().unwrap();
        cfg.team_code = Some(code);
        cfg.server = url.clone();
        cfg.save().map_err(|e| e.to_string())?;
    }
    let _ = state.cmd.send(HostCommand::Reconnect);
    Ok(url)
}

#[tauri::command]
fn new_password(state: State<'_, AppState>) -> String {
    let (pw, _) = one_time_password();
    *state.otp.lock().unwrap() = pw.clone();
    rebuild_auth(&state);
    pw
}

#[tauri::command]
fn set_server(state: State<'_, AppState>, server: String, name: String) -> Result<(), String> {
    {
        let mut cfg = state.cfg.lock().unwrap();
        cfg.server = server.trim().to_string();
        if !name.trim().is_empty() {
            cfg.name = name.trim().to_string();
        }
        cfg.save().map_err(|e| e.to_string())?;
    }
    let _ = state.cmd.send(HostCommand::Reconnect);
    Ok(())
}

/// Turn this PC into the server for the local network.
#[tauri::command]
fn enable_lan_server(state: State<'_, AppState>, enable: bool) -> Result<ServerState, String> {
    {
        let mut cfg = state.cfg.lock().unwrap();
        cfg.run_server = enable;
        if enable {
            cfg.server = format!("http://127.0.0.1:{LAN_PORT}");
        }
        cfg.save().map_err(|e| e.to_string())?;
    }
    if !enable {
        // Keeps running until the app restarts (sessions may depend on it).
        return Ok(state.server_state.lock().unwrap().clone());
    }
    let st = start_embedded_server(&state.server_state);
    let _ = state.cmd.send(HostCommand::Reconnect);
    Ok(st)
}

#[tauri::command]
fn set_options(state: State<'_, AppState>, ask_to_accept: bool) -> Result<(), String> {
    {
        let mut cfg = state.cfg.lock().unwrap();
        cfg.ask_to_accept = ask_to_accept;
        cfg.save().map_err(|e| e.to_string())?;
    }
    rebuild_auth(&state);
    Ok(())
}

fn main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" { Ok(()) } else { Err("Security settings are available only in the main window.".into()) }
}

fn factor_attempt(state: &AppState) -> Result<(), String> {
    let mut attempts = state.factor_attempts.lock().unwrap();
    attempts.retain(|time| time.elapsed().as_secs() < 300);
    if attempts.len() >= 8 { return Err("Too many attempts. Wait five minutes.".into()); }
    attempts.push(std::time::Instant::now());
    Ok(())
}

#[derive(Serialize)]
struct FactorSetup { secret: String, qr_base64: String }

#[tauri::command]
fn begin_two_factor(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<FactorSetup, String> {
    main_window(&window)?;
    let cfg = state.cfg.lock().unwrap();
    if cfg.two_factor.is_some() { return Err("Two-factor authentication is already enabled.".into()); }
    let factor = veyndesk_host::security::new_config();
    let generator = veyndesk_host::security::totp(&factor.secret, &cfg.name).map_err(|e| e.to_string())?;
    let qr_base64 = generator.to_qr_base64().map_err(|_| "Could not generate setup QR code".to_string())?;
    let result = FactorSetup { secret: factor.secret.clone(), qr_base64 };
    *state.pending_factor.lock().unwrap() = Some((factor, std::time::Instant::now()));
    Ok(result)
}

#[tauri::command]
fn cancel_two_factor(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    main_window(&window)?;
    *state.pending_factor.lock().unwrap() = None;
    Ok(())
}

#[tauri::command]
fn confirm_two_factor(window: tauri::WebviewWindow, state: State<'_, AppState>, code: String) -> Result<Vec<String>, String> {
    main_window(&window)?;
    factor_attempt(&state)?;
    let mut cfg = state.cfg.lock().unwrap();
    if cfg.two_factor.is_some() { return Err("Two-factor authentication is already enabled.".into()); }
    let mut pending = state.pending_factor.lock().unwrap();
    let Some((factor, since)) = pending.as_ref() else { return Err("Start setup first.".into()); };
    if since.elapsed().as_secs() > 600 { *pending = None; return Err("Setup expired. Start again.".into()); }
    let mut factor = factor.clone();
    if !veyndesk_host::security::consume_local(&mut factor, &code, veyndesk_host::security::now()) {
        return Err("Invalid code. Check the code and your phone's clock.".into());
    }
    let codes = veyndesk_host::security::recovery_codes(&mut factor);
    let mut next = cfg.clone();
    next.two_factor = Some(factor);
    next.save().map_err(|e| e.to_string())?;
    *cfg = next;
    *pending = None;
    // Invalidate password-only challenges issued before enrollment.
    let _ = state.cmd.send(HostCommand::Reconnect);
    Ok(codes)
}

#[tauri::command]
fn disable_two_factor(window: tauri::WebviewWindow, state: State<'_, AppState>, code: String) -> Result<(), String> {
    main_window(&window)?;
    factor_attempt(&state)?;
    let mut cfg = state.cfg.lock().unwrap();
    let mut next = cfg.clone();
    let factor = next.two_factor.as_mut().ok_or("Two-factor authentication is not enabled.")?;
    if !veyndesk_host::security::consume_local(factor, &code, veyndesk_host::security::now()) {
        return Err("Invalid or already used code. Use a fresh authenticator code or a recovery code.".into());
    }
    next.two_factor = None;
    next.save().map_err(|e| e.to_string())?;
    *cfg = next;
    Ok(())
}

#[tauri::command]
fn set_auto_lock(window: tauri::WebviewWindow, state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    main_window(&window)?;
    if enabled && !cfg!(target_os = "linux") { return Err("Auto-lock currently supports Linux hosts.".into()); }
    let mut cfg = state.cfg.lock().unwrap();
    let mut next = cfg.clone();
    next.lock_after_disconnect = enabled;
    next.save().map_err(|e| e.to_string())?;
    *cfg = next;
    Ok(())
}

#[tauri::command]
fn set_screen_blanking(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    if enabled && (!cfg!(target_os = "linux") || std::env::var("XDG_SESSION_TYPE").unwrap_or_default() != "x11") {
        return Err("Screen blanking requires Ubuntu on Xorg (X11).".into());
    }
    let mut cfg = state.cfg.lock().unwrap();
    let previous = cfg.blank_screen_on_connect;
    cfg.blank_screen_on_connect = enabled;
    if let Err(error) = cfg.save() {
        cfg.blank_screen_on_connect = previous;
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
fn set_permanent_password(state: State<'_, AppState>, password: Option<String>) -> Result<(), String> {
    {
        let mut cfg = state.cfg.lock().unwrap();
        match password {
            Some(p) if p.len() >= 8 => cfg.permanent = Some(Credentials::from_password(&p)),
            Some(_) => return Err("Use at least 8 characters.".into()),
            None => cfg.permanent = None,
        }
        cfg.save().map_err(|e| e.to_string())?;
    }
    rebuild_auth(&state);
    Ok(())
}

#[tauri::command]
fn set_start_at_login(enabled: bool) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("Start at login is currently supported on Linux only.".into());
    }
    let path = autostart_file()?;
    if enabled {
        if HostConfig::load_or_default().permanent.is_none() {
            return Err("Set a permanent access password before enabling start at login. The one-time password is not available while Veynremote is hidden.".into());
        }
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let exec = format!("\"{}\" --background", desktop_exec_argument(&executable.to_string_lossy()));
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Veynremote\nComment=Keep this computer available for remote connections\nExec={exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
        );
        let parent = path.parent().ok_or("Invalid autostart path.")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        std::fs::write(path, entry).map_err(|e| e.to_string())?;
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// Exit only after checking the local Veynremote access password.
/// The permanent password is verified from its stored salted hash. If no permanent
/// password is configured, the current run's one-time password is required.
#[tauri::command]
fn quit_app(app: tauri::AppHandle, state: State<'_, AppState>, password: String) -> Result<(), String> {
    let valid = {
        let cfg = state.cfg.lock().unwrap();
        if let Some(credentials) = &cfg.permanent {
            ct_eq(&credentials.secret, &password_secret(&credentials.salt, &password))
        } else {
            ct_eq(&state.otp.lock().unwrap(), &password)
        }
    };
    if !valid {
        return Err("Incorrect Veynremote password.".into());
    }
    state.allow_exit.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn respond(state: State<'_, AppState>, sid: u32, accept: bool) {
    let _ = state.cmd.send(HostCommand::Respond { sid, accept });
}

#[tauri::command]
fn disconnect_all(state: State<'_, AppState>) {
    let _ = state.cmd.send(HostCommand::DisconnectAll);
}

/// Opens the remote screen of partner `id` in a new window.
///
/// Must be `async`: creating a window from a synchronous command deadlocks on Windows
/// (the window stays white). The viewer page is bundled with the app, so a firewall or
/// wrong address shows a readable error instead of a blank window.
/// LAN servers found by broadcast (for first-run setup).
#[tauri::command]
async fn discover_servers() -> Vec<discovery::Answer> {
    discovery::find_servers().await
}

/// Opens the remote screen of a partner, given an ID or an IP address, in a new window.
///
/// * IP address → asks that PC directly (UDP 21119) for its ID and server.
/// * ID → first asks the LAN who has it (so IDs work even if PCs use different servers),
///   then falls back to the configured server (internet / VPS server).
///
/// Must be `async`: creating a window from a synchronous command deadlocks on Windows.
#[tauri::command]
async fn open_viewer(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let raw = id.trim().to_string();
    let configured = state.cfg.lock().unwrap().server.clone();
    let (server, id) = if raw.contains('.') {
        let host = raw.split(':').next().unwrap_or("").trim();
        let ip: std::net::IpAddr = host.parse().map_err(|_| format!("'{raw}' is not a valid IP address or ID"))?;
        let a = discovery::probe_ip(ip).await.ok_or_else(|| {
            format!("No Veynremote answered at {ip}. Make sure Veynremote is open there and allowed in Windows Firewall.")
        })?;
        let id = a.id.ok_or("That PC is not ready yet (no ID). Try again in a few seconds.")?;
        (a.server, id)
    } else {
        let id: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
        if id.len() < 6 {
            return Err("Enter a remote ID (9 digits) or an IP address".into());
        }
        // Fallback server if a direct connection is impossible: the partner's LAN server,
        // else our team server (a loopback server only knows this PC, so it's no fallback).
        let fallback = match discovery::find_id(&id).await {
            Some(a) => a.server,
            None if !is_loopback(&configured) => http_base(&configured),
            None => String::new(),
        };
        (fallback, id)
    };
    let p2p = !raw.contains('.');
    let server = if server.is_empty() { String::new() } else { http_base(&server) };
    let init = format!(
        "window.__VEYNDESK__ = {};",
        serde_json::json!({ "server": if server.is_empty() { serde_json::Value::Null } else { server.into() }, "id": id, "embedded": true, "p2p": p2p })
    );
    let label = format!("remote-{id}-{}", app.webview_windows().len());
    WebviewWindowBuilder::new(&app, label, WebviewUrl::App("viewer.html".into()))
        .initialization_script(&init)
        .title(format!("Veynremote — {id}"))
        .inner_size(1280.0, 800.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn is_loopback(server: &str) -> bool {
    server.trim().is_empty() || server.contains("127.0.0.1") || server.contains("localhost")
}

fn viewer_ws(server: &str) -> String {
    let mut c = HostConfig::load_or_default();
    c.server = server.to_string();
    c.ws_url().replace("/ws/host", "/ws/viewer")
}

/// Viewer window: deliver its WebRTC offer to partner `id` (via public signaling) and return the answer.
#[tauri::command]
async fn p2p_signal(id: String, sdp: String) -> Result<serde_json::Value, String> {
    let (sdp, relay) = veyndesk_host::p2p::request_answer(&id, sdp).await?;
    Ok(serde_json::json!({ "sdp": sdp, "relay": relay }))
}

fn main() {
    tracing_subscriber::fmt::init();
    veyndesk_host::init_platform();
    let _instance_lock = match acquire_instance_lock() {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            eprintln!("another Veynremote instance is already running for this user");
            return;
        }
        Err(error) => {
            eprintln!("could not acquire the Veynremote instance lock: {error}");
            return;
        }
    };
    let start_hidden = std::env::args().any(|arg| arg == "--background");

    let mut cfg = HostConfig::load_or_default();
    // Zero setup: every PC runs its own small server and is reachable peer-to-peer by ID.
    if cfg.server.trim().is_empty() {
        cfg.server = format!("http://127.0.0.1:{LAN_PORT}");
        cfg.run_server = true;
    }
    let _ = cfg.save();
    let (otp, _) = one_time_password();
    let mut auth = AuthConfig::build(&cfg, Some(&otp));
    auth.ask_on_one_time = cfg.ask_to_accept;
    let run_server = cfg.run_server;
    let auth = Arc::new(Mutex::new(auth));
    let cfg = Arc::new(Mutex::new(cfg));
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (ui_tx, mut ui_rx) = unbounded_channel::<UiEvent>();
    let id = Arc::new(Mutex::new(None));
    let status = Arc::new(Mutex::new("Starting…".to_string()));
    let server_state = Arc::new(Mutex::new(ServerState::Off));
    let lan = Arc::new(Mutex::new(discovery::LocalInfo::default()));

    let net = internet::Internet::new();
    let allow_exit = Arc::new(AtomicBool::new(false));
    let follow_tx = cmd_tx.clone();
    let net_exit = net.clone();
    let state = AppState {
        allow_exit: allow_exit.clone(),
        cfg: cfg.clone(),
        auth: auth.clone(),
        otp: Mutex::new(otp),
        pending_factor: Mutex::new(None),
        factor_attempts: Mutex::new(vec![]),
        id: id.clone(),
        status: status.clone(),
        cmd: cmd_tx,
        server_state: server_state.clone(),
        net: net.clone(),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_info,
            new_password,
            set_server,
            enable_lan_server,
            set_options,
            set_permanent_password,
            set_start_at_login,
            quit_app,
            set_screen_blanking,
            begin_two_factor,
            cancel_two_factor,
            confirm_two_factor,
            disable_two_factor,
            set_auto_lock,
            respond,
            disconnect_all,
            discover_servers,
            p2p_signal,
            enable_internet,
            join_team,
            open_viewer
        ])
        .setup(move |app| {
            let show_item = tauri::menu::MenuItem::with_id(app, "show", "Open Veynremote", true, None::<&str>)?;
            let quit_item = tauri::menu::MenuItem::with_id(app, "quit", "Quit Veynremote…", true, None::<&str>)?;
            let tray_menu = tauri::menu::Menu::with_items(app, &[&show_item, &quit_item])?;
            tauri::tray::TrayIconBuilder::new()
                .icon(app.default_window_icon().expect("default app icon").clone())
                .tooltip("Veynremote — available for connections")
                .menu(&tray_menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                            let _ = window.emit("request-quit", ());
                        }
                    }
                    _ => {}
                })
                .build(app)?;
            if run_server {
                start_embedded_server(&server_state);
            }
            // Server PC: internet tunnel. Other PCs: follow the team's current address.
            {
                let c = cfg.lock().unwrap().clone();
                if c.run_server {
                    // Free relay for when a direct (P2P) connection is impossible.
                    if c.internet {
                        net.set_code(c.team_code.clone());
                    }
                    net.start(LAN_PORT);
                }
                // Tell the P2P listener which public relay partners can fall back to.
                let (cfg4, net4) = (cfg.clone(), net.clone());
                std::thread::spawn(move || loop {
                    let server = cfg4.lock().unwrap().server.clone();
                    let relay = if is_loopback(&server) {
                        net4.url()
                    } else if server.starts_with("https://") {
                        Some(http_base(&server))
                    } else {
                        None
                    };
                    veyndesk_host::p2p::set_relay(relay);
                    std::thread::sleep(std::time::Duration::from_secs(3));
                });
                let cfg2 = cfg.clone();
                internet::spawn_republisher(net.clone(), move || cfg2.lock().unwrap().team_code.clone());
                let cfg3 = cfg.clone();
                std::thread::spawn(move || loop {
                    let (code, server, is_server) = {
                        let c = cfg3.lock().unwrap();
                        (c.team_code.clone(), c.server.clone(), c.run_server)
                    };
                    if let (Some(code), false) = (code, is_server) {
                        if let Ok(url) = internet::resolve(&code) {
                            if url != server {
                                {
                                    let mut c = cfg3.lock().unwrap();
                                    c.server = url;
                                    let _ = c.save();
                                }
                                let _ = follow_tx.send(HostCommand::Reconnect);
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(60));
                });
            }
            // Keep the LAN-discovery answer up to date and serve it.
            {
                let (l2, cfg, id, st) = (lan.clone(), cfg.clone(), id.clone(), server_state.clone());
                tauri::async_runtime::spawn(async move {
                    let lan = l2;
                    loop {
                        {
                            let c = cfg.lock().unwrap();
                            let mut l = lan.lock().unwrap();
                            l.id = id.lock().unwrap().clone().or_else(|| c.id.clone());
                            l.name = c.name.clone();
                            l.server = http_base(&c.server);
                            if c.server.trim().is_empty() {
                                l.server.clear();
                            }
                            l.server_port = matches!(*st.lock().unwrap(), ServerState::Running { .. }).then_some(LAN_PORT);
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                });
                tauri::async_runtime::spawn(discovery::responder(lan.clone()));
            }
            let (p2p_tx, p2p_rx) = tokio::sync::watch::channel(None);
            tauri::async_runtime::spawn(veyndesk_host::p2p::serve(p2p_rx));
            let p2p_cfg = cfg.clone();
            tauri::async_runtime::spawn(veyndesk_host::run(cfg, auth, ui_tx, cmd_rx, veyndesk_host::default_source()));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(ev) = ui_rx.recv().await {
                    match &ev {
                        UiEvent::Registered { id: new_id } => {
                            *id.lock().unwrap() = Some(new_id.clone());
                            // Listen for direct (P2P) partners on our ID, bridged to the server we're on.
                            let ws = viewer_ws(&p2p_cfg.lock().unwrap().server);
                            let _ = p2p_tx.send(Some((new_id.clone(), ws)));
                            *status.lock().unwrap() = "Ready".into();
                        }
                        UiEvent::Connecting { .. } => *status.lock().unwrap() = "Connecting…".into(),
                        UiEvent::Disconnected { reason } => *status.lock().unwrap() = format!("Offline: {reason}"),
                        UiEvent::ApprovalRequest { .. } => {
                            // Pop the window to the front so the user sees the prompt.
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.unminimize();
                                let _ = w.show();
                                let _ = w.set_focus();
                                let _ = w.request_user_attention(Some(tauri::UserAttentionType::Critical));
                            }
                        }
                        _ => {}
                    }
                    let _ = handle.emit("host-event", &ev);
                }
            });
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title(&format!("Veynremote {}", app.package_info().version));
                if !start_hidden {
                    let _ = w.show();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while starting Veynremote")
        .run(move |_, ev| {
            match ev {
                tauri::RunEvent::ExitRequested { api, .. } => {
                    // Closing the last window (or a desktop-environment exit request)
                    // must not silently stop the remote host. Only quit_app can set
                    // this flag after validating the local password.
                    if !allow_exit.load(Ordering::SeqCst) {
                        api.prevent_exit();
                    }
                }
                tauri::RunEvent::Exit => {
                    net_exit.stop(); // don't leave cloudflared running
                }
                _ => {}
            }
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn http_base() {
        assert_eq!(super::http_base("ws://1.2.3.4:8080"), "http://1.2.3.4:8080");
        assert_eq!(super::http_base("wss://desk.example.com/ws/host"), "https://desk.example.com");
        assert_eq!(super::http_base("desk.local:8080/"), "http://desk.local:8080");
    }
}
