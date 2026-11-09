//! One caller's visit: login or signup, then the menu loop.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use openbbs_core::VERSION;
use openbbs_core::config::Config;
use openbbs_core::text::expand_tokens;
use openbbs_core::users::{self, NewUser, User};

use crate::menu::{Action, Menu, MenuRunner};
use crate::nodes::{NodeGuard, Nodes};
use crate::store::{Store, blocking};
use crate::term::{Hangup, LineOpts, Term, TermResult};
use crate::theme::Theme;

/// Everything sessions share.
pub struct Bbs {
    pub config: Config,
    pub store: Store,
    pub nodes: Nodes,
}

impl Bbs {
    pub fn new(config: Config, store: Store) -> Arc<Self> {
        let nodes = Nodes::new(config.bbs.max_nodes);
        Arc::new(Self {
            config,
            store,
            nodes,
        })
    }

    pub fn theme(&self) -> Theme {
        Theme::load(&self.config.theme_dir()).unwrap_or_else(|e| {
            tracing::warn!("theme failed to load, using built-in colors: {e:#}");
            Theme::load(std::path::Path::new("")).expect("built-in theme loads")
        })
    }
}

/// Details about the connection, from the SSH layer.
pub struct Caller {
    pub peer: Option<SocketAddr>,
    /// The user name the SSH client logged in with.
    pub ssh_user: String,
    /// Set when SSH already checked the caller's handle and password.
    pub user: Option<User>,
}

const LOGIN_ATTEMPTS: usize = 3;

/// Run a caller's whole visit. Returns when they log off or hang up.
pub async fn run(bbs: Arc<Bbs>, mut term: Term, caller: Caller) {
    let Some(node) = bbs.nodes.claim(caller.peer) else {
        let _ = term.println("|12All nodes are busy. Please call back later.");
        return;
    };
    tracing::info!(node = node.number(), peer = ?caller.peer, "caller connected");
    let mut session = Session {
        bbs,
        node,
        user: None,
    };
    match session.visit(&mut term, &caller).await {
        Ok(()) => {}
        Err(e) => match e.downcast_ref::<Hangup>() {
            Some(Hangup::Idle) => {
                let _ = term.println("\r\n|12Idle too long. Disconnecting.");
            }
            Some(Hangup::Disconnected) => {}
            None => {
                tracing::error!(node = session.node.number(), "session error: {e:#}");
                let _ = term.println("\r\n|12Sorry, something went wrong. Disconnecting.");
            }
        },
    }
    tracing::info!(
        node = session.node.number(),
        user = session.user.as_ref().map(|u| u.handle.as_str()),
        "caller disconnected"
    );
}

pub(crate) struct Session {
    pub(crate) bbs: Arc<Bbs>,
    pub(crate) node: NodeGuard,
    pub(crate) user: Option<User>,
}

impl Session {
    async fn visit(&mut self, term: &mut Term, caller: &Caller) -> Result<()> {
        let theme = self.bbs.theme();
        self.show_screen(term, &theme, "welcome")?;
        self.show_screen(term, &theme, "welcome-info")?;
        term.println("")?;

        let user = match &caller.user {
            Some(user) => {
                term.println(&format!("|07Logged in as |15{}|07.", user.handle))?;
                user.clone()
            }
            None => match self.login(term).await? {
                Some(user) => user,
                None => {
                    term.println("|12Too many failed attempts. Goodbye.")?;
                    return Ok(());
                }
            },
        };
        let user = self
            .bbs
            .store
            .call(move |c| users::record_login(c, user.id))
            .await?;
        self.node.set_user(&user.handle, user.location.as_deref());
        term.println("")?;
        term.println(&format!(
            "|07Welcome, |15{}|07. This is your call number |15{}|07.",
            user.handle, user.times_called
        ))?;
        if let Some(last) = &user.last_login_at
            && user.times_called > 1
        {
            term.println(&format!("|07Last time you called: |15{last} UTC"))?;
        }
        self.user = Some(user);
        let (me, _) = self.me();
        let level = self.level();
        let (new_mail, new_posts) = self
            .bbs
            .store
            .call(move |c| {
                Ok((
                    openbbs_core::mail::unread_count(c, me)?,
                    openbbs_core::messages::total_new(c, me, level)?,
                ))
            })
            .await?;
        if new_posts > 0 {
            term.println(&format!(
                "|07There are |14{new_posts}|07 new messages on the boards."
            ))?;
        }
        if new_mail > 0 {
            term.println(&format!("|14You have {new_mail} new private mail."))?;
            if term.yes_no("|07Read your mail now?", true).await? {
                self.mailbox(term).await?;
            }
        } else {
            term.pause().await?;
        }

        self.menu_loop(term).await?;

        let theme = self.bbs.theme();
        term.clear()?;
        self.show_screen(term, &theme, "goodbye")?;
        term.println("")?;
        Ok(())
    }

    /// Ask for a handle (or NEW) and password. `None` after too many failures.
    async fn login(&mut self, term: &mut Term) -> Result<Option<User>> {
        for _ in 0..LOGIN_ATTEMPTS {
            let handle = term
                .ask(
                    "|07Enter your |15handle|07, or |11NEW|07 to sign up: ",
                    LineOpts::text(users::HANDLE_MAX),
                )
                .await?;
            if handle.is_empty() {
                continue;
            }
            if handle.eq_ignore_ascii_case("new") {
                if let Some(user) = self.signup(term).await? {
                    return Ok(Some(user));
                }
                continue;
            }
            let password = term.ask("|07Password: ", LineOpts::password()).await?;
            self.node.set_activity("Logging in");
            let lookup = handle.clone();
            let found = self
                .bbs
                .store
                .call(move |c| users::find_for_login(c, &lookup))
                .await?;
            let ok = match found {
                Some((user, hash)) => {
                    let valid = blocking(move || users::verify_password(&password, &hash)).await?;
                    valid.then_some(user)
                }
                None => {
                    // Spend the same time as a real check so handles can't be probed.
                    let _ = blocking(move || users::hash_password(&password)).await?;
                    None
                }
            };
            match ok {
                Some(user) => return Ok(Some(user)),
                None => {
                    tracing::info!(handle, "failed login");
                    term.println("|12Wrong handle or password.")?;
                }
            }
        }
        Ok(None)
    }

    /// New user signup. `None` if the caller backs out.
    async fn signup(&mut self, term: &mut Term) -> Result<Option<User>> {
        self.node.set_activity("New user signup");
        let theme = self.bbs.theme();
        self.show_screen(term, &theme, "newuser")?;

        let handle = loop {
            let handle = term
                .ask(
                    "|07Choose a handle |08(blank to cancel)|07: ",
                    LineOpts::text(users::HANDLE_MAX),
                )
                .await?;
            if handle.is_empty() {
                return Ok(None);
            }
            if let Err(e) = users::validate_handle(&handle) {
                term.println(&format!("|12{e}"))?;
                continue;
            }
            let check = handle.clone();
            if self
                .bbs
                .store
                .call(move |c| users::handle_taken(c, &check))
                .await?
            {
                term.println("|12That handle is already taken.")?;
                continue;
            }
            break handle;
        };

        let password = loop {
            let first = term
                .ask("|07Choose a password: ", LineOpts::password())
                .await?;
            if let Err(e) = users::validate_password(&first) {
                term.println(&format!("|12{e}"))?;
                continue;
            }
            let second = term.ask("|07Type it again: ", LineOpts::password()).await?;
            if first != second {
                term.println("|12Those didn't match. Try again.")?;
                continue;
            }
            break first;
        };

        let real_name = term
            .ask("|07Real name |08(optional)|07: ", LineOpts::text(40))
            .await?;
        let location = term
            .ask(
                "|07Location, like city or state |08(optional)|07: ",
                LineOpts::text(40),
            )
            .await?;
        term.println("")?;
        if !term.yes_no("|07Create this account?", true).await? {
            return Ok(None);
        }

        let new = NewUser {
            handle,
            password,
            real_name: Some(real_name).filter(|s| !s.is_empty()),
            location: Some(location).filter(|s| !s.is_empty()),
        };
        let pw = new.password.clone();
        let hash = blocking(move || users::hash_password(&pw)).await??;
        let created = self
            .bbs
            .store
            .call(move |c| users::create(c, &new, &hash))
            .await;
        match created {
            Ok(user) => {
                tracing::info!(handle = user.handle, level = user.access_level, "new user");
                term.println(&format!(
                    "|10Account created. Welcome aboard, |15{}|10!",
                    user.handle
                ))?;
                if user.is_sysop() {
                    term.println(
                        "|14You're the first caller, so you're the sysop. The Sysop Menu is yours.",
                    )?;
                }
                Ok(Some(user))
            }
            Err(e) => {
                term.println(&format!("|12Couldn't create the account: {e}"))?;
                Ok(None)
            }
        }
    }

    async fn menu_loop(&mut self, term: &mut Term) -> Result<()> {
        let mut stack = vec!["main".to_string()];
        while let Some(name) = stack.last().cloned() {
            let theme = self.bbs.theme();
            let menu = match theme.menu_source(&name)? {
                Some(src) => match Menu::parse(&src) {
                    Ok(menu) => menu,
                    Err(e) => {
                        tracing::error!(menu = name, "menu file is invalid: {e:#}");
                        term.println(&format!("|12The {name} menu has an error. Tell the sysop."))?;
                        term.pause().await?;
                        if stack.len() == 1 {
                            return Ok(());
                        }
                        stack.pop();
                        continue;
                    }
                },
                None => {
                    tracing::error!(menu = name, "menu file not found");
                    term.println(&format!("|12The {name} menu is missing. Tell the sysop."))?;
                    term.pause().await?;
                    if stack.len() == 1 {
                        return Ok(());
                    }
                    stack.pop();
                    continue;
                }
            };
            let menu = self.expand_menu(menu);
            self.node.set_activity(&format!("In {}", menu.title));
            let screen = match &menu.screen {
                Some(s) => theme.screen(s)?.map(|text| self.expand(&text)),
                None => None,
            };
            let runner = MenuRunner {
                menu: &menu,
                colors: &theme.colors,
                level: self.level(),
                screen,
            };
            let action = runner.choose(term).await?;
            match action {
                Action::Menu(next) => stack.push(next),
                Action::Back => {
                    if stack.len() > 1 {
                        stack.pop();
                    }
                }
                Action::Logoff => {
                    if term.yes_no("|07Log off now?", true).await? {
                        return Ok(());
                    }
                }
                Action::Who => self.who(term).await?,
                Action::UserList => self.user_list(term).await?,
                Action::UserInfo => self.user_info(term).await?,
                Action::Screen(name) => {
                    term.clear()?;
                    self.show_screen(term, &theme, &name)?;
                    term.pause().await?;
                }
                Action::Messages => self.message_areas(term).await?,
                Action::NewScan => self.new_scan(term).await?,
                Action::Mail => self.mailbox(term).await?,
                Action::SendMail => self.send_mail(term, None).await?,
                Action::Files => self.file_areas(term).await?,
                Action::RescanFiles => self.rescan_files(term).await?,
                Action::Doors => self.door_list(term).await?,
                Action::Door(id) => self.run_door(term, &id).await?,
                Action::SysopUsers => self.sysop_users(term).await?,
                Action::SysopMessageAreas => self.sysop_message_areas(term).await?,
                Action::SysopFileAreas => self.sysop_file_areas(term).await?,
                Action::ComingSoon(what) => {
                    let text = theme
                        .screen("comingsoon")?
                        .unwrap_or_else(|| "@FEATURE@ aren't open yet.".into());
                    let text = expand_tokens(&text, |t| (t == "FEATURE").then(|| what.clone()));
                    term.print(&self.expand(&text))?;
                    term.pause().await?;
                }
            }
        }
        Ok(())
    }

    async fn who(&mut self, term: &mut Term) -> TermResult<()> {
        term.clear()?;
        term.println("|11  Who's online")?;
        term.println(
            "|08  ────  ────────────────────────  ────────────────────  ───────────────────",
        )?;
        term.println("|03  Node  User                      Location              Doing")?;
        for info in self.bbs.nodes.online() {
            let mins = info.connected_at.elapsed().as_secs() / 60;
            term.println(&format!(
                "|15  {:>4}  |07{:<24}  {:<20}  |08{} ({}m)",
                info.node,
                info.handle.as_deref().unwrap_or("(logging in)"),
                info.location.as_deref().unwrap_or(""),
                info.activity,
                mins
            ))?;
        }
        term.println("")?;
        term.pause().await
    }

    async fn user_list(&mut self, term: &mut Term) -> Result<()> {
        let rows = self
            .bbs
            .store
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT handle, coalesce(location, ''), times_called, access_level
                     FROM users ORDER BY handle COLLATE NOCASE",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;
        let (_, height) = term.size();
        let page = usize::from(height.max(10)) - 6;
        let sysop = self.level() >= users::LEVEL_SYSOP;
        term.clear()?;
        let header = |term: &Term| -> TermResult<()> {
            term.println(&format!("|11  User list |08({} users)", rows.len()))?;
            term.println("|08  ────────────────────────  ────────────────────  ─────")?;
            Ok(())
        };
        header(term)?;
        for (i, (handle, location, calls, level)) in rows.iter().enumerate() {
            if i > 0 && i % page == 0 {
                term.pause().await?;
                term.clear()?;
                header(term)?;
            }
            let lvl = if sysop {
                format!("  |08lvl {level}")
            } else {
                String::new()
            };
            term.println(&format!(
                "|15  {handle:<24}  |07{location:<20}  |03{calls:>5}{lvl}"
            ))?;
        }
        term.println("")?;
        term.pause().await?;
        Ok(())
    }

    async fn user_info(&mut self, term: &mut Term) -> Result<()> {
        let Some(user) = &self.user else {
            return Ok(());
        };
        let id = user.id;
        let user = self
            .bbs
            .store
            .call(move |c| users::find_by_id(c, id))
            .await?
            .unwrap_or_else(|| user.clone());
        term.clear()?;
        term.println("|11  Your account")?;
        term.println("|08  ─────────────────────────────────────────")?;
        let field = |label: &str, value: &str| format!("|03  {label:<14}|15{value}");
        term.println(&field("Handle", &user.handle))?;
        term.println(&field(
            "Real name",
            user.real_name.as_deref().unwrap_or("-"),
        ))?;
        term.println(&field("Location", user.location.as_deref().unwrap_or("-")))?;
        term.println(&field("Access level", &user.access_level.to_string()))?;
        term.println(&field("Calls", &user.times_called.to_string()))?;
        term.println(&field("Member since", &format!("{} UTC", user.created_at)))?;
        term.println("")?;
        term.pause().await?;
        Ok(())
    }

    /// The logged-in user. Only called after login.
    pub(crate) fn me(&self) -> (i64, String) {
        self.user
            .as_ref()
            .map(|u| (u.id, u.handle.clone()))
            .unwrap_or((0, "Guest".into()))
    }

    pub(crate) fn level(&self) -> i64 {
        self.user.as_ref().map(|u| u.access_level).unwrap_or(0)
    }

    pub(crate) fn show_screen(&self, term: &Term, theme: &Theme, name: &str) -> Result<()> {
        if let Some(text) = theme.screen(name)? {
            term.print(&self.expand(&text))?;
        }
        Ok(())
    }

    fn expand_menu(&self, mut menu: Menu) -> Menu {
        menu.title = self.expand(&menu.title);
        menu.subtitle = self.expand(&menu.subtitle);
        menu.prompt = self.expand(&menu.prompt);
        menu
    }

    /// Fill in `@TOKENS@` such as @USER@ and @BBSNAME@.
    pub(crate) fn expand(&self, text: &str) -> String {
        let cfg = &self.bbs.config;
        expand_tokens(text, |token| {
            Some(match token {
                "BBSNAME" => cfg.bbs.name.clone(),
                "SYSOP" => cfg.bbs.sysop.clone(),
                "VERSION" => VERSION.to_string(),
                "NODE" => self.node.number().to_string(),
                "USER" => self
                    .user
                    .as_ref()
                    .map(|u| u.handle.clone())
                    .unwrap_or_else(|| "Guest".into()),
                "LEVEL" => self.level().to_string(),
                "ONLINE" => self.bbs.nodes.online().len().to_string(),
                _ => return None,
            })
        })
    }
}
