//! The SSH front end. Every connection that opens a shell gets a BBS session.
//!
//! Logging in:
//!
//! * `ssh yourhandle@host` asks for your BBS password at the SSH level and
//!   drops you straight at the main menu. This is also how SFTP (file
//!   transfers) logs in.
//! * Any other user name (`ssh new@host`, `ssh bbs@host`) is let in as a
//!   guest, the way a dial-up board answered every call, and the BBS asks for
//!   a handle or NEW.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use russh::keys::{Algorithm, PrivateKey, PublicKey, ssh_key::LineEnding};
use russh::server::{Auth, ChannelOpenHandle, Handle, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Pty};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};

use crate::session::{self, Bbs, Caller};
use crate::sftp::SftpSession;
use crate::store::blocking;
use crate::term::Term;
use openbbs_core::users::{self, User};

/// Load the host key, creating one on first run.
pub fn load_or_create_host_key(path: &Path) -> Result<PrivateKey> {
    if path.exists() {
        return russh::keys::load_secret_key(path, None)
            .with_context(|| format!("loading SSH host key {}", path.display()));
    }
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
        .map_err(|e| anyhow::anyhow!("generating host key: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pem = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("encoding host key: {e}"))?;
    std::fs::write(path, pem.as_bytes())
        .with_context(|| format!("writing SSH host key {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    tracing::info!(path = %path.display(), "created new SSH host key");
    Ok(key)
}

fn server_config(key: PrivateKey, idle_minutes: u32) -> russh::server::Config {
    let mut methods = MethodSet::empty();
    methods.push(MethodKind::None);
    methods.push(MethodKind::Password);
    russh::server::Config {
        methods,
        keys: vec![key],
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::from_secs(0)),
        // The BBS enforces its own idle limit; this is a backstop.
        inactivity_timeout: Some(Duration::from_secs(u64::from(idle_minutes) * 60 + 300)),
        nodelay: true,
        ..Default::default()
    }
}

/// Listen for SSH callers on an already-bound socket until the task is dropped.
pub async fn serve(bbs: Arc<Bbs>, key: PrivateKey, listener: TcpListener) -> Result<()> {
    let config = Arc::new(server_config(key, bbs.config.bbs.idle_minutes));
    let mut server = BbsServer { bbs };
    server.run_on_socket(config, &listener).await?;
    Ok(())
}

struct BbsServer {
    bbs: Arc<Bbs>,
}

impl Server for BbsServer {
    type Handler = Connection;

    fn new_client(&mut self, peer: Option<SocketAddr>) -> Connection {
        Connection {
            bbs: self.bbs.clone(),
            peer,
            ssh_user: String::new(),
            user: None,
            channel: None,
            channel_handle: None,
            input: None,
            size: watch::channel((80, 24)).0,
            started: false,
        }
    }

    fn handle_session_error(&mut self, error: anyhow::Error) {
        tracing::debug!("ssh session error: {error:#}");
    }
}

struct Connection {
    bbs: Arc<Bbs>,
    peer: Option<SocketAddr>,
    ssh_user: String,
    /// Set when the SSH login used a real handle and password.
    user: Option<User>,
    channel: Option<ChannelId>,
    /// The channel object, kept until we know whether it's a shell or SFTP.
    channel_handle: Option<Channel<Msg>>,
    input: Option<mpsc::Sender<Vec<u8>>>,
    size: watch::Sender<(u16, u16)>,
    started: bool,
}

fn clamp_size(cols: u32, rows: u32) -> (u16, u16) {
    let fix = |v: u32, default: u16| match v {
        0 => default,
        v => u16::try_from(v).unwrap_or(u16::MAX).clamp(20, 500),
    };
    (fix(cols, 80), fix(rows, 24))
}

impl Connection {
    async fn handle_exists(&self, user: &str) -> Result<bool> {
        if users::validate_handle(user).is_err() {
            return Ok(false);
        }
        let handle = user.to_string();
        self.bbs
            .store
            .call(move |c| users::handle_taken(c, &handle))
            .await
    }

    fn start_session(&mut self, channel: ChannelId, handle: Handle) {
        if self.started {
            return;
        }
        self.started = true;
        let (input_tx, input_rx) = mpsc::channel(64);
        self.input = Some(input_tx);
        let (output_tx, mut output_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let idle = Duration::from_secs(u64::from(self.bbs.config.bbs.idle_minutes) * 60);
        let term = Term::new(output_tx, input_rx, self.size.subscribe(), idle);
        let caller = Caller {
            peer: self.peer,
            ssh_user: self.ssh_user.clone(),
            user: self.user.clone(),
        };
        let bbs = self.bbs.clone();

        // Forward output to the SSH channel; hang up once the session is done
        // and everything it wrote has been sent.
        tokio::spawn(async move {
            while let Some(bytes) = output_rx.recv().await {
                if handle.data(channel, bytes).await.is_err() {
                    break;
                }
            }
            let _ = handle.exit_status_request(channel, 0).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
        tokio::spawn(session::run(bbs, term, caller));
    }
}

impl Handler for Connection {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        self.ssh_user = user.to_string();
        // Real handles must give their password; anyone else is a guest.
        if self.handle_exists(user).await? {
            Ok(Auth::reject())
        } else {
            Ok(Auth::Accept)
        }
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.ssh_user = user.to_string();
        let handle = user.to_string();
        let found = self
            .bbs
            .store
            .call(move |c| users::find_for_login(c, &handle))
            .await?;
        let Some((found, hash)) = found else {
            return Ok(Auth::Accept);
        };
        let password = password.to_string();
        if blocking(move || users::verify_password(&password, &hash)).await? {
            tracing::info!(user = found.handle, peer = ?self.peer, "ssh password login");
            self.user = Some(found);
            Ok(Auth::Accept)
        } else {
            tracing::info!(user, peer = ?self.peer, "ssh password rejected");
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(&mut self, user: &str, _key: &PublicKey) -> Result<Auth, Self::Error> {
        // No stored keys yet: real handles fall back to their password.
        self.ssh_user = user.to_string();
        if self.handle_exists(user).await? {
            Ok(Auth::reject())
        } else {
            Ok(Auth::Accept)
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        // One BBS session per connection.
        if self.channel.is_some() {
            return Ok(());
        }
        self.channel = Some(channel.id());
        self.channel_handle = Some(channel);
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.size.send_replace(clamp_size(cols, rows));
        session.channel_success(channel)?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.size.send_replace(clamp_size(cols, rows));
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        // The shell is driven through the session handle.
        self.channel_handle = None;
        self.start_session(channel, session.handle());
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let user = self.user.clone();
        match (name, user, self.channel_handle.take()) {
            ("sftp", Some(user), Some(handle)) if !self.started => {
                self.started = true;
                session.channel_success(channel)?;
                let sftp = SftpSession::new(self.bbs.clone(), user);
                russh_sftp::server::run(handle.into_stream(), sftp).await;
            }
            ("sftp", None, _) => {
                tracing::info!(
                    user = self.ssh_user,
                    "sftp refused: not logged in with a handle"
                );
                session.channel_failure(channel)?;
            }
            _ => session.channel_failure(channel)?,
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // No remote commands, just the BBS.
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn data(
        &mut self,
        _channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(input) = &self.input
            && input.send(data.to_vec()).await.is_err()
        {
            self.input = None;
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.input = None;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.input = None;
        Ok(())
    }
}
