use crate::greetd::{Cmd, Ev};
use crate::system::{self, Memory, Session, SystemInfo, User};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, ModifierKeyCode, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use std::{
    collections::VecDeque,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

pub const KEYS: [(u8, &str); 6] = [(1, "help"), (2, "users"), (3, "sessions"), (4, "power"), (7, "show pass"), (9, "ipc")];
pub const ROWS: usize = 3;
const MENU_KEYS: [u8; 4] = [1, 2, 3, 4];
pub const IDLE_DIM: Duration = Duration::from_secs(60);
const ANIM_MS: u128 = 300;

#[derive(Default)]
pub struct Input {
    pub text: String,
    pub cursor: usize,
}

impl Input {
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }
    fn byte(&self, ci: usize) -> usize {
        self.text.char_indices().nth(ci).map_or(self.text.len(), |(b, _)| b)
    }
    fn insert(&mut self, c: char) {
        let i = self.byte(self.cursor);
        self.text.insert(i, c);
        self.cursor += 1;
    }
    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let i = self.byte(self.cursor);
            self.text.remove(i);
        }
    }
    fn delete(&mut self) {
        if self.cursor < self.len() {
            let i = self.byte(self.cursor);
            self.text.remove(i);
        }
    }
    fn delete_word(&mut self) {
        while self.cursor > 0 && self.text.chars().nth(self.cursor - 1) == Some(' ') {
            self.backspace();
        }
        while self.cursor > 0 && self.text.chars().nth(self.cursor - 1) != Some(' ') {
            self.backspace();
        }
    }
    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = self.len();
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum MsgKind {
    Dim,
    Error,
    Busy,
}

#[derive(Clone, Copy, PartialEq)]
pub enum MenuKind {
    Users,
    Sessions,
    Power,
}

pub struct MenuItem {
    pub label: String,
    pub desc: String,
    pub current: bool,
}

pub struct Menu {
    pub kind: MenuKind,
    pub title: &'static str,
    pub items: Vec<MenuItem>,
    pub index: usize,
}

pub enum Overlay {
    None,
    Help,
    Menu(Menu),
}

pub struct Prompt;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    Text { row: usize, x0: u16, start: usize },
    Sess,
    SessPrev,
    SessNext,
    Key(usize),
    MenuItem(usize),
    Popup,
    Panel,
}

pub struct Hit {
    pub area: Rect,
    pub target: Target,
}

pub enum Exit {
    Started(Vec<String>),
    Quit,
}

pub struct App {
    pub sys: SystemInfo,
    pub users: Vec<User>,
    pub sessions: Vec<Session>,
    pub user: Input,
    pub pass: Input,
    pub si: usize,
    pub focus: usize,
    pub reveal: bool,
    pub peek: bool,
    pub caps: Option<bool>,
    pub msg: String,
    pub msg_kind: MsgKind,
    pub busy: bool,
    pub prompt: Option<Prompt>,
    pub overlay: Overlay,
    pub ipc_open: bool,
    pub ipc: VecDeque<(String, char, String)>,
    pub last_input: Instant,
    pub started_at: Instant,
    pub demo: bool,
    pub enhanced: bool,
    pam_error_shown: bool,
    backend_error: Option<String>,
    memory: Memory,
    cache_dir: PathBuf,
    tx: Sender<Cmd>,
    user_before_edit: String,
    pending_cmd: Vec<String>,
    pub exit: Option<Exit>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sys: SystemInfo,
        users: Vec<User>,
        sessions: Vec<Session>,
        cache_dir: PathBuf,
        tx: Sender<Cmd>,
        demo: bool,
        enhanced: bool,
        backend_error: Option<String>,
    ) -> Self {
        let memory = Memory::load(&cache_dir);
        let mut app = App {
            sys,
            users,
            sessions,
            user: Input::default(),
            pass: Input::default(),
            si: 0,
            focus: 0,
            reveal: false,
            peek: false,
            caps: None,
            msg: String::new(),
            msg_kind: MsgKind::Dim,
            busy: false,
            prompt: None,
            overlay: Overlay::None,
            ipc_open: false,
            ipc: VecDeque::new(),
            last_input: Instant::now(),
            started_at: Instant::now(),
            demo,
            enhanced,
            pam_error_shown: false,
            backend_error: backend_error.clone(),
            memory,
            cache_dir,
            tx,
            user_before_edit: String::new(),
            pending_cmd: vec![],
            exit: None,
        };
        let remembered = app.memory.user.clone().filter(|u| app.users.iter().any(|x| &x.name == u));
        let initial = remembered.or_else(|| (app.users.len() == 1).then(|| app.users[0].name.clone()));
        if let Some(u) = initial {
            app.user.set(&u);
            app.focus = 1;
        }
        app.user_before_edit = app.user.text.clone();
        app.apply_user_session();
        if let Some(e) = backend_error {
            app.set_msg(&e, MsgKind::Error);
        } else if demo {
            app.set_msg("Demo mode: sign in with password demo", MsgKind::Dim);
        }
        app.log('$', format!("greeter started on {}", app.sys.tty));
        app
    }

    // greetd runs every command through /bin/sh, so it is always there as a fallback.
    pub fn shell_of(&self, name: &str) -> String {
        match system::login_shell(name) {
            Some(shell) if system::is_executable(&shell) => shell,
            _ => "/bin/sh".into(),
        }
    }

    pub fn session_label(&self) -> String {
        self.sessions[self.si].display_name(&self.shell_of(self.user.text.trim()))
    }

    pub fn idle(&self) -> bool {
        self.last_input.elapsed() > IDLE_DIM
    }

    pub fn anim_step(&self) -> Option<u128> {
        (self.msg_kind == MsgKind::Busy).then(|| self.started_at.elapsed().as_millis() / ANIM_MS)
    }

    pub fn time_state(&self) -> (i64, Option<u128>, bool) {
        (chrono::Local::now().timestamp() / 60, self.anim_step(), self.idle())
    }

    pub fn until_next_change(&self) -> Duration {
        let ms_to_minute = 60_000 - chrono::Local::now().timestamp_millis().rem_euclid(60_000) as u64;
        let mut wait = Duration::from_millis(ms_to_minute);
        if self.anim_step().is_some() {
            let into = (self.started_at.elapsed().as_millis() % ANIM_MS) as u64;
            wait = wait.min(Duration::from_millis(ANIM_MS as u64 - into));
        }
        if let Some(left) = IDLE_DIM.checked_sub(self.last_input.elapsed()) {
            wait = wait.min(left);
        }
        wait
    }

    pub fn show_pass(&self) -> bool {
        self.reveal || self.peek
    }

    fn set_msg(&mut self, text: &str, kind: MsgKind) {
        self.msg = text.to_string();
        self.msg_kind = kind;
    }

    fn log(&mut self, dir: char, text: String) {
        let time = chrono::Local::now().format("%H:%M:%S").to_string();
        self.ipc.push_back((time, dir, text));
        while self.ipc.len() > 500 {
            self.ipc.pop_front();
        }
    }

    fn apply_user_session(&mut self) {
        let name = self.user.text.trim().to_string();
        if let Some(id) = self.memory.sessions.get(&name)
            && let Some(i) = self.sessions.iter().position(|s| &s.id == id)
        {
            self.si = i;
        }
    }

    fn set_focus(&mut self, i: usize) {
        let n = ROWS + KEYS.len();
        let i = i % n;
        if self.focus == 0 && i != 0 && self.user.text != self.user_before_edit {
            self.user_before_edit = self.user.text.clone();
            self.apply_user_session();
        }
        self.focus = i;
    }

    fn move_focus(&mut self, delta: i32) {
        let n = (ROWS + KEYS.len()) as i32;
        self.set_focus((self.focus as i32 + delta).rem_euclid(n) as usize);
    }

    fn cycle_session(&mut self, delta: i32) {
        let n = self.sessions.len() as i32;
        self.si = (self.si as i32 + delta).rem_euclid(n) as usize;
    }

    fn send(&mut self, cmd: Cmd) -> bool {
        if self.tx.send(cmd).is_ok() {
            return true;
        }
        self.busy = false;
        self.prompt = None;
        let why = self.backend_error.clone().unwrap_or_else(|| "Lost the connection to greetd".into());
        self.set_msg(&why, MsgKind::Error);
        false
    }

    fn hide_pass(&mut self) {
        self.pass.clear();
        self.reveal = false;
    }

    // The Caps Lock key's own press may report the state from before or after the toggle,
    // depending on the terminal: if it matches the known state, flip it. Other keys are exact.
    fn update_caps(&mut self, k: &KeyEvent) {
        if !self.enhanced {
            return;
        }
        let s = k.state.contains(KeyEventState::CAPS_LOCK);
        if k.code == KeyCode::CapsLock {
            if k.kind == KeyEventKind::Press {
                self.caps = Some(match self.caps {
                    Some(c) if s == c => !c,
                    _ => s,
                });
            }
        } else {
            self.caps = Some(s);
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        self.update_caps(&k);
        if let KeyCode::Modifier(m) = k.code {
            if matches!(m, ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt) {
                self.peek = k.kind != KeyEventKind::Release;
            }
            return;
        }
        if k.kind == KeyEventKind::Release {
            return;
        }
        self.last_input = Instant::now();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let back = k.code == KeyCode::BackTab || (k.code == KeyCode::Tab && k.modifiers.contains(KeyModifiers::SHIFT));
        let tab = k.code == KeyCode::Tab || k.code == KeyCode::BackTab;

        if ctrl && k.code == KeyCode::Char('c') && self.demo {
            self.exit = Some(Exit::Quit);
            return;
        }
        if let KeyCode::F(n) = k.code {
            self.fkey(n);
            return;
        }

        if let Overlay::Menu(m) = &mut self.overlay {
            let n = m.items.len();
            let kind = m.kind;
            match k.code {
                _ if tab => self.next_menu(menu_key(kind), if back { -1 } else { 1 }),
                KeyCode::Down | KeyCode::Char('j') => m.index = (m.index + 1) % n,
                KeyCode::Up | KeyCode::Char('k') => m.index = (m.index + n - 1) % n,
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let i = m.index;
                    self.overlay = Overlay::None;
                    self.pick(kind, i);
                }
                KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
                KeyCode::Char(d @ '1'..='9') if (d as usize - '0' as usize) <= n => {
                    self.overlay = Overlay::None;
                    self.pick(kind, d as usize - '1' as usize);
                }
                _ => {}
            }
            return;
        }
        if let Overlay::Help = self.overlay {
            match k.code {
                _ if tab => self.next_menu(1, if back { -1 } else { 1 }),
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => self.overlay = Overlay::None,
                _ => {}
            }
            return;
        }

        if k.code == KeyCode::Esc {
            if self.busy || self.prompt.is_some() {
                self.prompt = None;
                if self.send(Cmd::Cancel) {
                    self.busy = true;
                    self.set_msg("Cancelling", MsgKind::Busy);
                }
            } else if self.ipc_open {
                self.ipc_open = false;
            } else {
                self.hide_pass();
                self.set_msg("", MsgKind::Dim);
            }
            return;
        }
        if tab {
            self.move_focus(if back { -1 } else { 1 });
            return;
        }

        if self.focus >= ROWS {
            let last = ROWS + KEYS.len() - 1;
            match k.code {
                KeyCode::Enter | KeyCode::Char(' ') => self.fkey(KEYS[self.focus - ROWS].0),
                KeyCode::Right | KeyCode::Char('l') => self.focus = if self.focus == last { ROWS } else { self.focus + 1 },
                KeyCode::Left | KeyCode::Char('h') => self.focus = if self.focus == ROWS { last } else { self.focus - 1 },
                KeyCode::Up => self.focus = ROWS - 1,
                _ => {}
            }
            return;
        }

        if self.busy {
            return;
        }
        // greetd already has the username and session; only the answer can still change
        if self.prompt.is_some() && self.focus != 1 {
            if k.code == KeyCode::Enter {
                self.submit();
            } else if k.code == KeyCode::Down {
                self.set_focus(1);
            }
            return;
        }
        match k.code {
            KeyCode::Down => return self.set_focus((self.focus + 1).min(ROWS)),
            KeyCode::Up => return self.set_focus(self.focus.saturating_sub(1)),
            KeyCode::Enter => {
                if self.focus == 0 {
                    if self.user.text.trim().is_empty() {
                        self.set_msg("Enter a username", MsgKind::Error);
                    } else {
                        self.set_msg("", MsgKind::Dim);
                        self.set_focus(1);
                    }
                } else {
                    self.submit();
                }
                return;
            }
            _ => {}
        }

        if self.focus == 2 {
            match k.code {
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => self.cycle_session(1),
                KeyCode::Left | KeyCode::Char('h') => self.cycle_session(-1),
                _ => {}
            }
            return;
        }

        let caps_swap = self.enhanced && self.caps == Some(true);
        let field = if self.focus == 0 { &mut self.user } else { &mut self.pass };
        let mut typed = true;
        match k.code {
            KeyCode::Char('u') if ctrl => field.clear(),
            KeyCode::Char('w') if ctrl => field.delete_word(),
            KeyCode::Char('a') if ctrl => field.cursor = 0,
            KeyCode::Char('e') if ctrl => field.cursor = field.len(),
            KeyCode::Char(c) if !ctrl => {
                // the kitty protocol reports keys without Caps Lock applied
                let c = if caps_swap && c.is_alphabetic() { swap_case(c) } else { c };
                field.insert(c);
            }
            KeyCode::Backspace => field.backspace(),
            KeyCode::Delete => field.delete(),
            KeyCode::Left => field.cursor = field.cursor.saturating_sub(1),
            KeyCode::Right => field.cursor = (field.cursor + 1).min(field.len()),
            KeyCode::Home => field.cursor = 0,
            KeyCode::End => field.cursor = field.len(),
            _ => typed = false,
        }
        if typed && self.msg_kind == MsgKind::Error && self.prompt.is_none() {
            self.set_msg("", MsgKind::Dim);
        }
        if self.focus == 1 && self.pass.text.is_empty() {
            self.reveal = false;
        }
    }

    /// Returns whether the screen needs redrawing.
    pub fn on_mouse(&mut self, m: MouseEvent, hits: &[Hit]) -> bool {
        let at = hits
            .iter()
            .rev()
            .find(|h| h.area.contains(Position { x: m.column, y: m.row }))
            .map(|h| h.target);
        self.last_input = Instant::now();
        match m.kind {
            MouseEventKind::Moved => {
                if let (Overlay::Menu(menu), Some(Target::MenuItem(i))) = (&mut self.overlay, at)
                    && menu.index != i
                {
                    menu.index = i;
                    return true;
                }
                false
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d = if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
                if let Overlay::Menu(menu) = &mut self.overlay {
                    let n = menu.items.len() as i32;
                    menu.index = (menu.index as i32 + d).rem_euclid(n) as usize;
                    return true;
                }
                let on_sess = matches!(at, Some(Target::Sess | Target::SessPrev | Target::SessNext));
                if matches!(self.overlay, Overlay::None) && !self.busy && self.prompt.is_none() && (on_sess || self.focus == 2) {
                    self.cycle_session(d);
                    return true;
                }
                false
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.click(at, m.column);
                true
            }
            _ => false,
        }
    }

    fn click(&mut self, at: Option<Target>, column: u16) {
        if !matches!(self.overlay, Overlay::None) {
            match at {
                Some(Target::MenuItem(i)) => {
                    if let Overlay::Menu(menu) = &self.overlay {
                        let kind = menu.kind;
                        self.overlay = Overlay::None;
                        self.pick(kind, i);
                    }
                }
                Some(Target::Popup) => {}
                _ => self.overlay = Overlay::None,
            }
            return;
        }
        match at {
            Some(Target::Key(i)) => self.fkey(KEYS[i].0),
            Some(_) if self.busy => {}
            Some(Target::Text { row: 0, .. } | Target::Sess | Target::SessPrev | Target::SessNext) if self.prompt.is_some() => {}
            Some(Target::Text { row, x0, start }) => {
                self.set_focus(row);
                let field = if row == 0 { &mut self.user } else { &mut self.pass };
                field.cursor = (start + column.saturating_sub(x0) as usize).min(field.len());
            }
            Some(Target::Sess) => {
                if self.focus == 2 {
                    self.cycle_session(1);
                }
                self.set_focus(2);
            }
            Some(Target::SessPrev) => {
                self.set_focus(2);
                self.cycle_session(-1);
            }
            Some(Target::SessNext) => {
                self.set_focus(2);
                self.cycle_session(1);
            }
            _ => {}
        }
    }

    fn fkey(&mut self, n: u8) {
        match n {
            1 => self.overlay = Overlay::Help,
            2 if !self.busy && self.prompt.is_none() => self.users_menu(),
            3 if !self.busy && self.prompt.is_none() => self.sessions_menu(),
            4 => self.power_menu(),
            7 => self.reveal = !self.reveal,
            9 => self.ipc_open = !self.ipc_open,
            _ => {}
        }
    }

    fn next_menu(&mut self, cur: u8, dir: i32) {
        let len = MENU_KEYS.len() as i32;
        let mut i = MENU_KEYS.iter().position(|&k| k == cur).unwrap_or(0) as i32;
        for _ in 0..len {
            i = (i + dir).rem_euclid(len);
            let k = MENU_KEYS[i as usize];
            if !((self.busy || self.prompt.is_some()) && (k == 2 || k == 3)) {
                self.fkey(k);
                return;
            }
        }
    }

    fn users_menu(&mut self) {
        let cur = self.user.text.trim().to_string();
        let mut items: Vec<MenuItem> = self
            .users
            .iter()
            .map(|u| MenuItem { label: u.name.clone(), desc: format!("uid {} · {}", u.uid, u.shell), current: u.name == cur })
            .collect();
        items.push(MenuItem { label: "other…".into(), desc: "type a username".into(), current: false });
        let index = self.users.iter().position(|u| u.name == cur).unwrap_or(0);
        self.overlay = Overlay::Menu(Menu { kind: MenuKind::Users, title: "users", items, index });
    }

    fn sessions_menu(&mut self) {
        let shell = self.shell_of(self.user.text.trim());
        let items = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, s)| MenuItem {
                label: s.display_name(&shell),
                desc: format!("{} · {}", s.tag(), s.source),
                current: i == self.si,
            })
            .collect();
        self.overlay = Overlay::Menu(Menu { kind: MenuKind::Sessions, title: "sessions", items, index: self.si });
    }

    fn power_menu(&mut self) {
        let items = [("Suspend", "suspend"), ("Reboot", "reboot"), ("Power off", "poweroff")]
            .iter()
            .map(|(l, a)| MenuItem { label: l.to_string(), desc: format!("systemctl {a}"), current: false })
            .collect();
        self.overlay = Overlay::Menu(Menu { kind: MenuKind::Power, title: "power", items, index: 0 });
    }

    fn pick(&mut self, kind: MenuKind, i: usize) {
        match kind {
            MenuKind::Users if i >= self.users.len() => {
                self.user.clear();
                self.user_before_edit.clear();
                self.focus = 0;
            }
            MenuKind::Users => {
                let name = self.users[i].name.clone();
                if name != self.user.text.trim() {
                    self.hide_pass();
                }
                self.user.set(&name);
                self.user_before_edit = name;
                self.apply_user_session();
                self.set_msg("", MsgKind::Dim);
                self.focus = 1;
            }
            MenuKind::Sessions => self.si = i,
            MenuKind::Power => self.power(["suspend", "reboot", "poweroff"][i]),
        }
    }

    fn power(&mut self, action: &str) {
        self.log('$', format!("systemctl {action}"));
        if self.demo {
            self.set_msg(&format!("Demo: would run systemctl {action}"), MsgKind::Dim);
            return;
        }
        match Command::new("systemctl").arg(action).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
                self.set_msg(&format!("Running systemctl {action}"), MsgKind::Busy);
            }
            Err(e) => self.set_msg(&format!("systemctl {action} failed: {e}"), MsgKind::Error),
        }
    }

    fn submit(&mut self) {
        if self.prompt.take().is_some() {
            let answer = self.pass.text.clone();
            self.hide_pass();
            if self.send(Cmd::Reply(Some(answer))) {
                self.busy = true;
                self.set_msg("Authenticating", MsgKind::Busy);
            }
            return;
        }
        let name = self.user.text.trim().to_string();
        if name.is_empty() {
            self.set_msg("Enter a username", MsgKind::Error);
            return self.set_focus(0);
        }
        if self.pass.text.is_empty() {
            self.set_msg("Enter your password", MsgKind::Error);
            return self.set_focus(1);
        }
        let session = &self.sessions[self.si];
        let shell = self.shell_of(&name);
        let cmd = session.command(&shell);
        let env = session.env();
        self.pending_cmd = cmd.clone();
        self.busy = true;
        self.pam_error_shown = false;
        self.set_msg("Authenticating", MsgKind::Busy);
        let password = self.pass.text.clone();
        self.send(Cmd::Login { user: name, password, cmd, env });
    }

    pub fn on_backend(&mut self, ev: Ev) {
        match ev {
            Ev::Log(dir, text) => self.log(dir, text),
            Ev::Prompt { secret, text } => {
                self.busy = false;
                self.hide_pass();
                self.reveal = !secret;
                self.focus = 1;
                self.set_msg(text.trim(), MsgKind::Dim);
                self.prompt = Some(Prompt);
            }
            Ev::Info(text) => self.set_msg(text.trim(), MsgKind::Dim),
            Ev::PamError(text) => {
                self.pam_error_shown = true;
                self.set_msg(text.trim(), MsgKind::Error);
            }
            Ev::AuthFailed => {
                self.busy = false;
                self.prompt = None;
                self.hide_pass();
                self.focus = 1;
                if !self.pam_error_shown {
                    self.set_msg("Login incorrect", MsgKind::Error);
                }
            }
            Ev::Failed(desc) => {
                self.busy = false;
                self.prompt = None;
                self.set_msg(&desc, MsgKind::Error);
            }
            Ev::Cancelled => {
                self.busy = false;
                self.prompt = None;
                self.hide_pass();
                self.set_msg("Cancelled", MsgKind::Dim);
            }
            Ev::Started => {
                let name = self.user.text.trim().to_string();
                self.memory.user = Some(name.clone());
                self.memory.sessions.insert(name, self.sessions[self.si].id.clone());
                self.memory.save(&self.cache_dir);
                self.set_msg(&format!("Starting {}", self.session_label()), MsgKind::Busy);
                self.exit = Some(Exit::Started(std::mem::take(&mut self.pending_cmd)));
            }
        }
    }
}

fn menu_key(kind: MenuKind) -> u8 {
    match kind {
        MenuKind::Users => 2,
        MenuKind::Sessions => 3,
        MenuKind::Power => 4,
    }
}

fn swap_case(c: char) -> char {
    if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) } else { c.to_uppercase().next().unwrap_or(c) }
}
