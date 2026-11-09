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

    // Lightbar: arrow down three times to File Areas, Enter.
    first.send("\x1b[B\x1b[B\x1b[B\r").await;
    first.expect("File areas aren't open yet").await;
    first.send(" ").await;
    first.expect("Main »").await;

    // Post the first message in General.
    first.send("m").await;
    first.expect("Message Areas").await;
    first.send("\r").await;
    first.expect("No messages yet").await;
    first.send("p").await;
    first.expect("To (Enter for All)").await;
    first.send("\r").await;
    first.expect("Subject:").await;
    first.send("Hello world\r").await;
    first.expect("Type your message").await;
    first.send("First post!\r/s\r").await;
    first.expect("Posted in General").await;
    first.send(" ").await;
    first.expect("Hello world").await;
    first.send("q").await;
    first.expect("Message Areas").await;
    first.send("q").await;
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

    // A second caller signs up while the first is still on.
    let second = Caller::connect(addr, "newbie").await;
    second.expect("NEW").await;
    second.send("new\r").await;
    second.expect("Choose a handle").await;
    second.send("Reader\r").await;
    second.expect("Choose a password").await;
    second.send("secret99\r").await;
    second.expect("Type it again").await;
    second.send("secret99\r").await;
    second.expect("Real name").await;
    second.send("\r").await;
    second.expect("Location").await;
    second.send("\r").await;
    second.expect("Create this account?").await;
    second.send("\r").await;
    second.expect("Welcome aboard, Reader").await;
    second.expect("There are 1 new messages").await;
    second.send(" ").await;
    let menu = second.expect("Main »").await;
    assert!(
        !menu.contains("Sysop Menu"),
        "regular callers don't see the sysop menu"
    );

    // New scan finds the sysop's post; reply to it with a quote.
    second.send("n").await;
    second.expect("Scanning General: 1 new").await;
    second.send(" ").await;
    second.expect("Message 1 of 1").await;
    second.expect("First post!").await;
    second.send("r").await;
    second.expect("Quote the original message?").await;
    second.send("\r").await;
    second.expect("Subject: Re: Hello world").await;
    second.expect("C> First post!").await;
    second.send("Thanks!\r/s\r").await;
    second.expect("Posted in General").await;
    second.send(" ").await;
    second.send("q").await;
    second.expect("Keep scanning other areas?").await;
    second.send("n").await;
    second.expect("Main »").await;

    // Send the sysop private mail.
    second.send("e").await;
    second.expect("Your mailbox is empty").await;
    second.send("s").await;
    second.expect("To (handle").await;
    second.send("nobody\r").await;
    second.expect("no user by that name").await;
    second.send("clayton\r").await;
    second.expect("Subject:").await;
    second.send("Hi sysop\r").await;
    second.expect("Type your message").await;
    second.send("Thanks for the board\r/s\r").await;
    second.expect("Sent to Clayton").await;
    second.send(" ").await;
    second.expect("Private Mail").await;
    second.send("q").await;
    second.expect("Main »").await;
    second.send("g").await;
    second.expect("Log off now?").await;
    second.send("y").await;
    second.expect("Thanks for calling Test Board, Reader").await;
    second.wait_closed().await;

    // The sysop reads the mail and sees the reply on the board.
    first.send("q").await;
    first.expect("Main »").await;
    first.send("e").await;
    first.expect("1 letters, 1 unread").await;
    first.send("\r").await;
    first.expect("Thanks for the board").await;
    first.send("q").await;
    first.expect("1 letters, 0 unread").await;
    first.send("q").await;
    first.expect("Main »").await;
    first.send("m").await;
    first.expect("Message Areas").await;
    first.send("\r").await;
    first.expect("Re: Hello world").await;
    first.send("qq").await;
    first.expect("Main »").await;

    // Log in again as the sysop from a third connection: the SSH user name
    // skips the handle prompt, and a wrong password is refused.
    let third = Caller::connect(addr, "Clayton").await;
    third.expect("Handle: Clayton").await;
    third.send("wrongpass\r").await;
    third.expect("Wrong handle or password").await;
    third.expect("NEW").await;
    third.send("Clayton\r").await;
    third.expect("Password").await;
    third.send("hunter42\r").await;
    third.expect("call number 2").await;
    drop(third);

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
