//! Real host + server + viewer handshake, isolated from user settings and displays.
#![cfg(target_os = "linux")]
use futures_util::{SinkExt, StreamExt};
use std::{process::{Child, Command, Stdio}, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use veyndesk_host::{config::{Credentials, HostConfig}, security};
use veyndesk_protocol::*;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
struct HostChild(Child);
impl Drop for HostChild { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
fn start_host(home: &std::path::Path) -> HostChild {
    HostChild(Command::new(env!("CARGO_BIN_EXE_veyndesk-host"))
        .arg("--no-one-time").env("XDG_CONFIG_HOME", home).env("VEYNDESK_TEST_PATTERN", "1")
        .env("RUST_LOG", "off").stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap())
}
async fn control(socket: &mut Socket) -> ServerToViewer {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
                Some(Ok(_)) => {},
                _ => panic!("viewer connection closed before a response"),
            }
        }
    }).await.expect("viewer response timeout")
}
async fn wait_online(addr: std::net::SocketAddr, data: &std::path::Path, expected: bool) {
    for _ in 0..100 {
        let online = std::fs::read_to_string(data.join("server/admin_token")).ok().and_then(|token|
            ureq::get(&format!("http://{addr}/api/hosts")).timeout(Duration::from_secs(1))
                .set("Authorization", &format!("Bearer {}", token.trim())).call().ok())
            .and_then(|response| serde_json::from_reader::<_, Vec<serde_json::Value>>(response.into_reader()).ok())
            .is_some_and(|hosts| hosts.iter().any(|host| host["online"] == true));
        if online == expected { return; }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("host online state did not become {expected}");
}
async fn challenge(url: &str) -> (Socket, String, String, bool) {
    let (mut socket, _) = connect_async(url).await.unwrap();
    socket.send(Message::Text(serde_json::to_string(&ViewerControl::Connect { id: "123456789".into() }).unwrap().into())).await.unwrap();
    match control(&mut socket).await {
        ServerToViewer::Challenge { salt, nonce, two_factor } => (socket, salt, nonce, two_factor),
        error => panic!("expected challenge, got {error:?}"),
    }
}
async fn login(url: &str, password: &str, code: Option<&str>) -> ServerToViewer {
    let (mut socket, salt, nonce, enabled) = challenge(url).await;
    assert!(enabled);
    let secret = password_secret(&salt, password);
    let factor = code.map(|code| second_factor_proof(&secret, &nonce, &sha256_hex(&security::normalize_code(code).unwrap())));
    let auth = ViewerControl::Auth { proof: auth_proof(&secret, &nonce), second_factor_proof: factor };
    socket.send(Message::Text(serde_json::to_string(&auth).unwrap().into())).await.unwrap();
    control(&mut socket).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enforces_mfa_replay_recovery_and_rate_limits_through_real_server() {
    let temp = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(veyndesk_server::serve_on(listener, temp.path().join("server")));
    let mut factor = security::new_config();
    let codes = security::recovery_codes(&mut factor);
    let code = security::totp(&factor.secret, "test").unwrap().generate_current().to_string();
    let cfg: HostConfig = serde_json::from_value(serde_json::json!({
        "server": format!("http://{addr}"), "id": "123456789", "key": random_token(32),
        "name": "Isolated MFA test", "permanent": Credentials::from_password("test-password"),
        "ask_to_accept": false, "two_factor": factor,
        "blank_screen_on_connect": false, "lock_after_disconnect": false
    })).unwrap();
    let config_dir = temp.path().join("Veyndesk"); std::fs::create_dir(&config_dir).unwrap();
    let config_file = config_dir.join("host.json");
    std::fs::write(&config_file, serde_json::to_vec(&cfg).unwrap()).unwrap();
    let url = format!("ws://{addr}/ws/viewer");
    let mut host = start_host(temp.path());
    wait_online(addr, temp.path(), true).await;
    assert!(matches!(login(&url, "test-password", None).await, ServerToViewer::Rejected { .. }));
    assert!(matches!(login(&url, "wrong-password", Some(&code)).await, ServerToViewer::Rejected { .. }));
    assert!(matches!(login(&url, "test-password", Some(&code)).await, ServerToViewer::Accepted { .. }));
    assert!(matches!(login(&url, "test-password", Some(&code)).await, ServerToViewer::Rejected { .. }));
    host.0.kill().unwrap(); host.0.wait().unwrap();
    wait_online(addr, temp.path(), false).await;
    host = start_host(temp.path());
    wait_online(addr, temp.path(), true).await;
    assert!(matches!(login(&url, "test-password", Some(&code)).await, ServerToViewer::Rejected { .. }));
    assert!(matches!(login(&url, "test-password", Some(&codes[0])).await, ServerToViewer::Accepted { .. }));
    assert!(matches!(login(&url, "test-password", Some(&codes[0])).await, ServerToViewer::Rejected { .. }));
    let persisted: HostConfig = serde_json::from_slice(&std::fs::read(&config_file).unwrap()).unwrap();
    assert_eq!(persisted.two_factor.unwrap().recovery_hashes.len(), 9);
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&config_file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // Fill the remaining failure allowance, then even a fresh recovery code
    // must not bypass the lockout.
    for _ in 0..6 { assert!(matches!(login(&url, "test-password", None).await, ServerToViewer::Rejected { .. })); }
    let (mut socket, _) = connect_async(&url).await.unwrap();
    socket.send(Message::Text(serde_json::to_string(&ViewerControl::Connect { id: "123456789".into() }).unwrap().into())).await.unwrap();
    assert!(matches!(control(&mut socket).await, ServerToViewer::Rejected { .. }));
    drop(host);
    server.abort();
}
