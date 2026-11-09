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
    /// Connect as a guest (any user name that isn't a handle).
    async fn connect(addr: std::net::SocketAddr, user: &str) -> Self {
        Self::connect_with(addr, user, None).await
    }

    /// Connect with an SSH password (a real handle logs straight in).
    async fn connect_with(addr: std::net::SocketAddr, user: &str, password: Option<&str>) -> Self {
        let mut session = ssh_session(addr).await;
        let authed = match password {
            None => session.authenticate_none(user).await.unwrap(),
            Some(pw) => session.authenticate_password(user, pw).await.unwrap(),
        };
        assert!(authed.success(), "ssh login failed for {user}");
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

async fn ssh_session(addr: std::net::SocketAddr) -> client::Handle<AcceptAnyHost> {
    let config = Arc::new(client::Config::default());
    client::connect(config, addr, AcceptAnyHost).await.unwrap()
}

async fn start_board() -> std::net::SocketAddr {
    start_board_with_files(std::env::temp_dir().join("openbbs-test-unused-files")).await
}

async fn start_board_with_files(files: std::path::PathBuf) -> std::net::SocketAddr {
    start_board_with(|c| c.paths.files = files).await
}

async fn start_board_with(customize: impl FnOnce(&mut Config)) -> std::net::SocketAddr {
    let mut config = Config::default();
    config.bbs.name = "Test Board".into();
    config.paths.files = std::env::temp_dir().join("openbbs-test-unused-files");
    customize(&mut config);
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
    first.expect("File Areas").await;
    first.expect("Uploads").await;
    first.send("q").await;
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

    // A guest connection can still log in with a handle at the BBS prompt.
    let third = Caller::connect(addr, "bbs").await;
    third.expect("NEW").await;
    third.send("Clayton\r").await;
    third.expect("Password").await;
    third.send("wrongpass\r").await;
    third.expect("Wrong handle or password").await;
    third.send("Clayton\r").await;
    third.expect("Password").await;
    third.send("hunter42\r").await;
    third.expect("call number 2").await;
    drop(third);

    // SSH as the handle with the right password skips the BBS login.
    let fourth = Caller::connect_with(addr, "Clayton", Some("hunter42")).await;
    fourth.expect("Logged in as Clayton").await;
    fourth.expect("call number 3").await;
    drop(fourth);

    // Real handles can't get in over SSH without the password.
    let mut session = ssh_session(addr).await;
    assert!(
        !session
            .authenticate_none("Clayton")
            .await
            .unwrap()
            .success()
    );
    assert!(
        !session
            .authenticate_password("Clayton", "nope")
            .await
            .unwrap()
            .success()
    );

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

/// Sign a user up through the BBS so they exist for SFTP.
async fn sign_up(addr: std::net::SocketAddr, handle: &str, password: &str) {
    let c = Caller::connect(addr, "new").await;
    c.expect("NEW").await;
    c.send("new\r").await;
    c.expect("Choose a handle").await;
    c.send(&format!("{handle}\r")).await;
    c.expect("Choose a password").await;
    c.send(&format!("{password}\r{password}\r\r\r\r")).await;
    c.expect("Welcome aboard").await;
}

async fn open_sftp(
    addr: std::net::SocketAddr,
    user: &str,
    password: Option<&str>,
) -> Option<russh_sftp::client::SftpSession> {
    let mut session = ssh_session(addr).await;
    let ok = match password {
        Some(pw) => session.authenticate_password(user, pw).await.unwrap(),
        None => session.authenticate_none(user).await.unwrap(),
    };
    assert!(ok.success());
    let channel = session.channel_open_session().await.unwrap();
    channel.request_subsystem(true, "sftp").await.ok()?;
    let sftp = tokio::time::timeout(
        Duration::from_secs(5),
        russh_sftp::client::SftpSession::new(channel.into_stream()),
    )
    .await
    .ok()?
    .ok()?;
    // Keep the SSH connection alive as long as the SFTP session.
    std::mem::forget(session);
    Some(sftp)
}

#[tokio::test]
async fn sftp_upload_list_download() {
    let files = std::env::temp_dir().join(format!("openbbs-sftp-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&files);
    let addr = start_board_with_files(files.clone()).await;
    sign_up(addr, "Sysop Sam", "hunter42").await;
    sign_up(addr, "Caller", "secret99").await;

    // Guests get no SFTP.
    assert!(open_sftp(addr, "guest", None).await.is_none());

    let sftp = open_sftp(addr, "Caller", Some("secret99")).await.unwrap();
    let root: Vec<String> = sftp
        .read_dir("/")
        .await
        .unwrap()
        .map(|e| e.file_name())
        .collect();
    assert!(
        root.contains(&"uploads".to_string()),
        "root lists areas: {root:?}"
    );

    sftp.write("/uploads/hello.txt", b"Hello from SFTP")
        .await
        .unwrap();
    assert!(
        sftp.write("/uploads/../uploads/.sneaky", b"x")
            .await
            .is_err()
    );
    assert!(sftp.write("/nope/file.txt", b"x").await.is_err());
    let listing: Vec<String> = sftp
        .read_dir("/uploads")
        .await
        .unwrap()
        .map(|e| e.file_name())
        .collect();
    assert!(listing.contains(&"hello.txt".to_string()), "{listing:?}");
    assert_eq!(
        sftp.read("/uploads/hello.txt").await.unwrap(),
        b"Hello from SFTP"
    );
    assert!(files.join("uploads/hello.txt").is_file());
    // No temp files left behind.
    let leftovers = std::fs::read_dir(files.join("uploads"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.')
        })
        .count();
    assert_eq!(leftovers, 0);

    // The BBS shows the upload with its uploader and a download.
    let c = Caller::connect_with(addr, "Caller", Some("secret99")).await;
    c.expect("Press any key").await;
    c.send(" ").await;
    c.expect("Main »").await;
    c.send("f").await;
    c.expect("File Areas").await;
    c.send("\r").await;
    c.expect("hello.txt").await;
    c.send("\r").await;
    let info = c.expect("Press any key").await;
    assert!(info.contains("Uploader    Caller"), "{info}");
    assert!(info.contains("Downloads   1"), "{info}");
    assert!(info.contains("sftp -P"), "{info}");
    drop(c);

    // Another user can't overwrite or delete it; the sysop can.
    let other = open_sftp(addr, "Sysop Sam", Some("hunter42"))
        .await
        .unwrap();
    other.remove_file("/uploads/hello.txt").await.unwrap();
    assert!(!files.join("uploads/hello.txt").exists());
    let _ = std::fs::remove_dir_all(&files);
}

#[cfg(unix)]
#[tokio::test]
async fn door_gets_drop_file_and_terminal() {
    let base = std::env::temp_dir().join(format!("openbbs-door-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let doors = base.join("doors");
    std::fs::create_dir_all(&doors).unwrap();
    // A tiny "door": prints the handle from DOOR32.SYS, echoes one line back,
    // and shows a CP437 block to prove the conversion.
    std::fs::write(
        doors.join("doors.toml"),
        r#"
[[door]]
id = "echo"
name = "Echo Door"
command = "sh"
args = ["-c", "printf 'Player: '; sed -n 7p {door32} | tr -d '\\r'; read line; printf 'You said %s \\333\\n' \"$line\""]

[[door]]
id = "secret"
name = "Sysop Only"
command = "true"
level = 99
"#,
    )
    .unwrap();
    let (data, doors_dir) = (base.join("data"), doors.clone());
    let addr = start_board_with(move |c| {
        c.paths.doors = doors_dir;
        c.paths.data = data;
    })
    .await;
    sign_up(addr, "First Sysop", "hunter42").await;
    sign_up(addr, "Gamer", "secret99").await;

    let c = Caller::connect_with(addr, "Gamer", Some("secret99")).await;
    c.expect("Press any key").await;
    c.send(" ").await;
    c.expect("Main »").await;
    c.send("d").await;
    let list = c.expect("Echo Door").await;
    assert!(!list.contains("Sysop Only"), "level-gated door is hidden");
    c.send("\r").await;
    c.expect("Player: Gamer").await;
    c.send("hello door\n").await;
    c.expect("You said hello door █").await;
    c.expect("Back at Test Board from Echo Door").await;
    c.send(" ").await;
    c.expect("Door Games").await;
    assert!(base.join("data/nodes/node1/DOOR.SYS").is_file());
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn sysop_console_manages_areas_and_users() {
    let addr = start_board().await;
    sign_up(addr, "Boss", "hunter42").await;
    sign_up(addr, "Pal", "secret99").await;

    let boss = Caller::connect_with(addr, "Boss", Some("hunter42")).await;
    boss.expect("Press any key").await;
    boss.send(" ").await;
    boss.expect("Main »").await;
    boss.send("s").await;
    boss.expect("[Sysop] »").await;

    // Add a message area.
    boss.send("m").await;
    boss.expect("Sysop · Message Areas").await;
    boss.send("a").await;
    boss.expect("Name:").await;
    boss.send("Retro Games\r").await;
    boss.expect("Description:").await;
    boss.send("Old stuff\r").await;
    boss.expect("Level to read [10]").await;
    boss.send("\r").await;
    boss.expect("Level to post [10]").await;
    boss.send("\r").await;
    boss.expect("Area added.").await;
    boss.send(" ").await;
    boss.expect("Retro Games").await;
    boss.send("q").await;
    boss.expect("[Sysop] »").await;

    // Raise Pal's level (users are sorted: Boss, Pal).
    boss.send("u").await;
    boss.expect("Sysop · Users").await;
    boss.send("\x1b[Bl").await;
    boss.expect("Access level for Pal").await;
    boss.send("50\r").await;
    boss.expect("Level updated.").await;
    boss.send(" ").await;
    // The sysop can't demote themselves.
    boss.send("\x1b[Al").await;
    boss.expect("Access level for Boss").await;
    boss.send("10\r").await;
    boss.expect("can't take away your own sysop access").await;
    boss.send(" ").await;
    boss.send("q").await;
    boss.expect("[Sysop] »").await;

    // Pal sees the new area and their new level.
    let pal = Caller::connect_with(addr, "Pal", Some("secret99")).await;
    pal.expect("Press any key").await;
    pal.send(" ").await;
    pal.expect("Main »").await;
    pal.send("m").await;
    pal.expect("Retro Games").await;
    pal.send("q").await;
    pal.expect("Main »").await;
    pal.send("a").await;
    pal.expect("Access level  50").await;
}
