//! End-to-end: a real SSH client signs up, logs in, and walks the menus.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use openbbs::session::Bbs;
use openbbs::ssh;
use openbbs::store::Store;
use openbbs_core::config::Config;
use openbbs_core::text::strip_ansi;
use russh::ChannelMsg;
use russh::client;
use russh::keys::{Algorithm, PrivateKey, PublicKeyOrCertificate};
use tokio::net::TcpListener;

struct AcceptAnyHost;

impl client::Handler for AcceptAnyHost {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// A connected caller: send keys, wait for text on screen.
struct Caller {
    writer: russh::ChannelWriteHalf<client::Msg>,
    screen: Arc<Mutex<String>>,
    closed: Arc<Mutex<bool>>,
    _session: client::Handle<AcceptAnyHost>,
}

impl Caller {
    async fn connect(addr: std::net::SocketAddr, user: &str) -> Self {
        let config = Arc::new(client::Config::default());
        let mut session = client::connect(config, addr, AcceptAnyHost).await.unwrap();
        assert!(session.authenticate_none(user).await.unwrap().success());
        let channel = session.channel_open_session().await.unwrap();
        let (mut reader, writer) = channel.split();
        writer
            .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
            .await
            .unwrap();
        writer.request_shell(false).await.unwrap();

        let screen = Arc::new(Mutex::new(String::new()));
        let closed = Arc::new(Mutex::new(false));
        let (s, c) = (screen.clone(), closed.clone());
        tokio::spawn(async move {
            while let Some(msg) = reader.wait().await {
                match msg {
                    ChannelMsg::Data { data } => {
                        s.lock().unwrap().push_str(&String::from_utf8_lossy(&data));
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            *c.lock().unwrap() = true;
        });
        Self {
            writer,
            screen,
            closed,
            _session: session,
        }
    }

    async fn send(&self, text: &str) {
        self.writer
            .data_bytes(text.as_bytes().to_vec())
            .await
            .unwrap();
    }

    /// Wait until `needle` appears on screen, then clear what's been seen.
    async fn expect(&self, needle: &str) -> String {
        for _ in 0..200 {
            {
                let mut screen = self.screen.lock().unwrap();
                let plain = strip_ansi(&screen);
                if let Some(pos) = plain.find(needle) {
                    let seen = plain[..pos + needle.len()].to_string();
                    // Keep anything after the match for the next expect.
                    let rest = plain[pos + needle.len()..].to_string();
                    *screen = rest;
                    return seen;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let screen = strip_ansi(&self.screen.lock().unwrap());
        panic!("timed out waiting for {needle:?}; screen was:\n{screen}");
    }

    async fn wait_closed(&self) {
        for _ in 0..200 {
            if *self.closed.lock().unwrap() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("server did not hang up");
    }
}

async fn start_board() -> std::net::SocketAddr {
    let mut config = Config::default();
    config.bbs.name = "Test Board".into();
    // Use the built-in theme.
    config.paths.themes = std::env::temp_dir().join("openbbs-no-such-theme-dir");
    let store = Store::open_in_memory().unwrap();
    let bbs = Bbs::new(config, store);
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ssh::serve(bbs, key, listener));
    addr
}

#[tokio::test]
async fn sign_up_log_in_and_use_menus() {
    let addr = start_board().await;

    // First caller signs up and becomes the sysop.
    let first = Caller::connect(addr, "anyone").await;
    first.expect("Welcome to Test Board").await;
    first.expect("NEW").await;
    first.send("new\r").await;
    first.expect("Choose a handle").await;
    first.send("x\r").await;
    first.expect("Handles must be").await;
    first.send("Clayton\r").await;
    first.expect("Choose a password").await;
    first.send("hunter42\r").await;
    first.expect("Type it again").await;
    first.send("hunter42\r").await;
    first.expect("Real name").await;
    first.send("\r").await;
    first.expect("Location").await;
    first.send("WSL\r").await;
    first.expect("Create this account?").await;
    first.send("\r").await;
    first.expect("you're the sysop").await;
    first.expect("Press any key").await;
    first.send(" ").await;
    let menu = first.expect("Main »").await;
    assert!(menu.contains("Message Boards"));
    assert!(menu.contains("Sysop Menu"), "sysop sees the sysop menu");

    // Lightbar: arrow down twice to File Areas, Enter.
    first.send("\x1b[B\x1b[B\r").await;
    first.expect("File areas aren't open yet").await;
    first.send(" ").await;
    first.expect("Main »").await;

    // Hotkey into the sysop menu, then who's online.
    first.send("s").await;
    first.expect("[Sysop] »").await;
    first.send("w").await;
    let who = first.expect("Press any key").await;
    assert!(
        who.contains("Clayton"),
        "who's online lists the caller: {who}"
    );
    first.send(" ").await;
    first.expect("[Sysop] »").await;

    // Second caller connects as their handle while the first is still on:
    // the SSH user name skips the handle prompt.
    let second = Caller::connect(addr, "Clayton").await;
    second.expect("Handle: Clayton").await;
    second.send("wrongpass\r").await;
    second.expect("Wrong handle or password").await;
    second.expect("NEW").await;
    second.send("Clayton\r").await;
    second.expect("Password").await;
    second.send("hunter42\r").await;
    second.expect("call number 2").await;
    second.send(" ").await;
    second.expect("Main »").await;
    second.send("g").await;
    second.expect("Log off now?").await;
    second.send("y").await;
    second.expect("Thanks for calling Test Board").await;
    second.wait_closed().await;

    // Back on the first node: leave the sysop menu and log off.
    first.send("q").await;
    first.expect("Main »").await;
    first.send("g").await;
    first.expect("Log off now?").await;
    first.send("\r").await;
    first.expect("NO CARRIER").await;
    first.wait_closed().await;
}

#[tokio::test]
async fn full_board_turns_callers_away() {
    let mut config = Config::default();
    config.bbs.max_nodes = 1;
    let bbs = Bbs::new(config, Store::open_in_memory().unwrap());
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ssh::serve(bbs, key, listener));

    let first = Caller::connect(addr, "a").await;
    first.expect("NEW").await;
    let second = Caller::connect(addr, "b").await;
    second.expect("All nodes are busy").await;
    second.wait_closed().await;
}
