use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub struct SystemInfo {
    pub host: String,
    pub os: String,
    pub kernel: String,
    pub tty: String,
}

impl SystemInfo {
    pub fn read() -> Self {
        let read = |p: &str| fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default();
        let os = fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("NAME=").map(|v| v.trim_matches('"').to_string()))
            })
            .unwrap_or_else(|| "Linux".into());
        let tty = read("/sys/class/tty/tty0/active");
        SystemInfo {
            host: read("/proc/sys/kernel/hostname"),
            os,
            kernel: read("/proc/sys/kernel/osrelease"),
            tty: if tty.is_empty() { "tty".into() } else { tty },
        }
    }
}

pub struct User {
    pub name: String,
    pub uid: u32,
    pub shell: String,
}

pub fn read_users() -> Vec<User> {
    let (min, max) = uid_range();
    let mut users: Vec<User> = fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid: u32 = f[2].parse().ok()?;
            let shell = f[6].to_string();
            let can_login = !shell.ends_with("nologin") && !shell.ends_with("/false");
            (uid >= min && uid <= max && can_login).then(|| User { name: f[0].into(), uid, shell })
        })
        .collect();
    users.sort_by_key(|u| u.uid);
    users
}

pub fn login_shell(name: &str) -> Option<String> {
    shell_in(&fs::read_to_string("/etc/passwd").ok()?, name)
}

fn shell_in(passwd: &str, name: &str) -> Option<String> {
    passwd.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        // passwd(5): an empty shell field means /bin/sh
        (f.len() >= 7 && f[0] == name).then(|| if f[6].is_empty() { "/bin/sh".to_string() } else { f[6].to_string() })
    })
}

fn uid_range() -> (u32, u32) {
    let defs = fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let get = |key: &str, default: u32| {
        defs.lines()
            .find_map(|l| {
                let mut it = l.split_whitespace();
                (it.next() == Some(key)).then(|| it.next()?.parse().ok()).flatten()
            })
            .unwrap_or(default)
    };
    (get("UID_MIN", 1000), get("UID_MAX", 60000))
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SessionKind {
    Wayland,
    X11,
    Shell,
}

pub struct Session {
    pub id: String,
    pub name: String,
    pub kind: SessionKind,
    pub exec: Vec<String>,
    pub desktop_names: String,
    pub source: String,
}

impl Session {
    pub fn tag(&self) -> &'static str {
        match self.kind {
            SessionKind::Wayland => "wayland",
            SessionKind::X11 => "x11",
            SessionKind::Shell => "tty",
        }
    }

    pub fn display_name(&self, shell: &str) -> String {
        match self.kind {
            SessionKind::Shell => format!("Shell ({})", shell.rsplit('/').next().unwrap_or(shell)),
            _ => self.name.clone(),
        }
    }

    pub fn command(&self, shell: &str) -> Vec<String> {
        match self.kind {
            SessionKind::Wayland => self.exec.clone(),
            SessionKind::X11 => ["startx", "/usr/bin/env"]
                .iter()
                .map(|s| s.to_string())
                .chain(self.exec.iter().cloned())
                .collect(),
            SessionKind::Shell => vec![shell.to_string(), "-l".into()],
        }
    }

    pub fn env(&self) -> Vec<String> {
        let mut env = vec![format!("XDG_SESSION_TYPE={}", self.tag())];
        if self.kind != SessionKind::Shell {
            let stem = self.id.trim_end_matches(".desktop");
            let current = if self.desktop_names.is_empty() { stem.to_string() } else { self.desktop_names.replace(';', ":").trim_end_matches(':').to_string() };
            env.push(format!("XDG_SESSION_DESKTOP={stem}"));
            env.push(format!("XDG_CURRENT_DESKTOP={current}"));
        }
        env
    }
}

pub fn read_sessions() -> Vec<Session> {
    let mut sessions = Vec::new();
    for (dir, kind) in [
        ("/usr/share/wayland-sessions", SessionKind::Wayland),
        ("/usr/share/xsessions", SessionKind::X11),
    ] {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect();
        paths.sort();
        for path in paths {
            if let Some(s) = parse_desktop(&path, kind) {
                sessions.push(s);
            }
        }
    }
    sessions.push(Session {
        id: "shell".into(),
        name: "Shell".into(),
        kind: SessionKind::Shell,
        exec: vec![],
        desktop_names: String::new(),
        source: "login shell".into(),
    });
    sessions
}

fn parse_desktop(path: &Path, kind: SessionKind) -> Option<Session> {
    let text = fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let (mut name, mut exec, mut try_exec, mut names) = (None, None, None, String::new());
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "Name" => name = Some(v.trim().to_string()),
            "Exec" => exec = Some(v.trim().to_string()),
            "TryExec" => try_exec = Some(v.trim().to_string()),
            "DesktopNames" => names = v.trim().to_string(),
            "Hidden" | "NoDisplay" if v.trim() == "true" => return None,
            _ => {}
        }
    }
    if let Some(t) = try_exec
        && !is_executable(&t)
    {
        return None;
    }
    Some(Session {
        id: path.file_name()?.to_string_lossy().into_owned(),
        name: name?,
        kind,
        exec: split_exec(&exec?),
        desktop_names: names,
        source: path.display().to_string(),
    })
}

pub fn is_executable(prog: &str) -> bool {
    let ok = |p: &Path| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if prog.contains('/') {
        return ok(Path::new(prog));
    }
    std::env::var("PATH")
        .unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into())
        .split(':')
        .any(|dir| ok(&Path::new(dir).join(prog)))
}

fn split_exec(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut has) = (false, false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out.retain(|a| !(a.len() == 2 && a.starts_with('%')));
    out
}

#[derive(Serialize, Deserialize, Default)]
pub struct Memory {
    pub user: Option<String>,
    #[serde(default)]
    pub sessions: HashMap<String, String>,
}

impl Memory {
    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(dir.join("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        let _ = fs::create_dir_all(dir);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(dir.join("state.json"), json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_splitting() {
        assert_eq!(split_exec("/usr/bin/startplasma-wayland"), ["/usr/bin/startplasma-wayland"]);
        assert_eq!(split_exec(r#"sh -c "echo hi there" %U"#), ["sh", "-c", "echo hi there"]);
        assert_eq!(
            split_exec("/usr/lib/plasma-dbus-run-session-if-needed /usr/bin/startplasma-wayland"),
            ["/usr/lib/plasma-dbus-run-session-if-needed", "/usr/bin/startplasma-wayland"]
        );
    }

    #[test]
    fn login_shell_lookup() {
        let passwd = "root:x:0:0::/root:/usr/bin/bash\nibra:x:1000:1000::/home/ibra:/usr/bin/fish\nold:x:1001:1001::/home/old:\n";
        assert_eq!(shell_in(passwd, "ibra").as_deref(), Some("/usr/bin/fish"));
        assert_eq!(shell_in(passwd, "root").as_deref(), Some("/usr/bin/bash"));
        assert_eq!(shell_in(passwd, "old").as_deref(), Some("/bin/sh"));
        assert_eq!(shell_in(passwd, "nobody"), None);
        assert_eq!(shell_in(passwd, ""), None);
    }
}
