use crate::app::{App, Hit, MsgKind, Overlay, Target, KEYS, ROWS};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear},
    Frame,
};

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Color,
    pub panel: Color,
    pub fg: Color,
    pub dim: Color,
    pub faint: Color,
    pub amber: Color,
    pub red: Color,
    pub green: Color,
}

impl Theme {
    pub fn truecolor() -> Self {
        Theme {
            bg: Color::Rgb(0x10, 0x12, 0x11),
            panel: Color::Rgb(0x15, 0x18, 0x16),
            fg: Color::Rgb(0xCD, 0xD2, 0xCD),
            dim: Color::Rgb(0x5C, 0x65, 0x5F),
            faint: Color::Rgb(0x26, 0x2B, 0x28),
            amber: Color::Rgb(0xE8, 0xC0, 0x7D),
            red: Color::Rgb(0xE2, 0x7B, 0x7B),
            green: Color::Rgb(0x9C, 0xC9, 0x8A),
        }
    }

    pub fn ansi() -> Self {
        Theme {
            bg: Color::Reset,
            panel: Color::Reset,
            fg: Color::White,
            dim: Color::DarkGray,
            faint: Color::DarkGray,
            amber: Color::Yellow,
            red: Color::Red,
            green: Color::Green,
        }
    }

    pub fn pick() -> Self {
        if std::env::var("TERM").is_ok_and(|t| t == "linux") { Self::ansi() } else { Self::truecolor() }
    }

    fn dimmed(&self) -> Self {
        let f = |c: Color, k: f32| match c {
            Color::Rgb(r, g, b) => {
                let (br, bg, bb) = (0x10 as f32, 0x12 as f32, 0x11 as f32);
                let mix = |v: u8, base: f32| (base + (v as f32 - base) * k) as u8;
                Color::Rgb(mix(r, br), mix(g, bg), mix(b, bb))
            }
            other => if k < 0.5 { Color::DarkGray } else { other },
        };
        Theme {
            fg: f(self.fg, 0.32),
            dim: f(self.dim, 0.45),
            faint: f(self.faint, 0.7),
            amber: f(self.amber, 0.32),
            red: f(self.red, 0.32),
            green: f(self.green, 0.32),
            ..*self
        }
    }
}

fn st(fg: Color) -> Style {
    Style::default().fg(fg)
}

fn put(buf: &mut Buffer, clip: Rect, x: u16, y: u16, text: &str, style: Style) -> u16 {
    let clip = clip.intersection(buf.area);
    if y < clip.y || y >= clip.bottom() || x < clip.x || x >= clip.right() {
        return x;
    }
    buf.set_stringn(x, y, text, (clip.right() - x) as usize, style).0
}

fn width(s: &str) -> u16 {
    s.chars().count() as u16
}

const SEG: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F]; // seven-segment bits a..g
const UP: u8 = 1;
const DN: u8 = 2;
const LT: u8 = 4;
const RT: u8 = 8;

fn box_char(m: u8) -> char {
    match m {
        3 => '│',
        12 => '─',
        10 => '╭',
        6 => '╮',
        9 => '╰',
        5 => '╯',
        11 => '├',
        7 => '┤',
        14 => '┬',
        13 => '┴',
        15 => '┼',
        1 => '╵',
        2 => '╷',
        4 => '╴',
        8 => '╶',
        _ => ' ',
    }
}

fn digit(d: u8, w: usize, h: usize) -> Vec<String> {
    let seg = SEG[d as usize];
    let mut m = vec![vec![0u8; w]; h];
    let mid = h / 2;
    let hline = |m: &mut Vec<Vec<u8>>, r: usize| {
        for x in 0..w - 1 {
            m[r][x] |= RT;
            m[r][x + 1] |= LT;
        }
    };
    let vline = |m: &mut Vec<Vec<u8>>, c: usize, r0: usize, r1: usize| {
        for y in r0..r1 {
            m[y][c] |= DN;
            m[y + 1][c] |= UP;
        }
    };
    if seg & 0x01 != 0 { hline(&mut m, 0) }
    if seg & 0x02 != 0 { vline(&mut m, w - 1, 0, mid) }
    if seg & 0x04 != 0 { vline(&mut m, w - 1, mid, h - 1) }
    if seg & 0x08 != 0 { hline(&mut m, h - 1) }
    if seg & 0x10 != 0 { vline(&mut m, 0, mid, h - 1) }
    if seg & 0x20 != 0 { vline(&mut m, 0, 0, mid) }
    if seg & 0x40 != 0 { hline(&mut m, mid) }
    m.into_iter().map(|row| row.into_iter().map(box_char).collect()).collect()
}

fn big_time(hhmm: &str, w: usize, h: usize) -> Vec<String> {
    let mut rows = vec![String::new(); h];
    let mid = h / 2;
    for (i, ch) in hhmm.chars().enumerate() {
        if i > 0 {
            rows.iter_mut().for_each(|r| r.push_str("  "));
        }
        if ch == ':' {
            for (y, r) in rows.iter_mut().enumerate() {
                r.push(if y + 1 == mid || y == mid + 1 { '•' } else { ' ' });
            }
        } else if let Some(d) = ch.to_digit(10) {
            for (r, line) in rows.iter_mut().zip(digit(d as u8, w, h)) {
                r.push_str(&line);
            }
        }
    }
    rows
}

pub fn draw(f: &mut Frame, app: &App, theme: Theme, hits: &mut Vec<Hit>) {
    hits.clear();
    let area = f.area();
    let popup = !matches!(app.overlay, Overlay::None);
    let t = if popup || app.idle() { theme.dimmed() } else { theme };
    let buf = f.buffer_mut();
    Block::default().style(Style::default().bg(t.bg).fg(t.fg)).render_ref(area, buf);

    let pad_x = (area.width * 5 / 100).max(3);
    let pad_y = (area.height * 6 / 100).max(1);
    let inner = Rect {
        x: area.x + pad_x,
        y: area.y + pad_y,
        width: area.width.saturating_sub(pad_x * 2),
        height: area.height.saturating_sub(pad_y * 2),
    };
    if inner.width < 20 || inner.height < 8 {
        put(buf, area, 0, 0, "window too small", st(t.dim));
        return;
    }
    let now = chrono::Local::now();

    let host = format!("{} · {} · greetd{}", app.sys.host, app.sys.tty, if app.demo { " (demo)" } else { "" });
    put(buf, inner, inner.x, inner.y, &host, st(t.dim));
    let date = now.format("%a, %-d %b %Y").to_string();
    put(buf, inner, inner.right().saturating_sub(width(&date)), inner.y, &date, st(t.dim));

    // widest row (sess) is 36 columns; 40 leaves room for the key bar beside it
    let form_w = inner.width.min(40);
    let keys_w: u16 = KEYS.iter().map(|(n, l)| width(&format!("F{n} {l}")) + 2).sum::<u16>() - 2;
    let keys_inline = inner.width >= form_w + 4 + keys_w;
    let bottom_h = if keys_inline { 5 } else { 7 };
    let fy = inner.bottom().saturating_sub(bottom_h);

    let (top, bot) = (inner.y + 2, fy.saturating_sub(1));
    let avail = bot.saturating_sub(top);
    let hhmm = now.format("%H:%M").to_string();
    let osline = format!("{} · {}", app.sys.os, app.sys.kernel);
    let sizes = [(8usize, 7usize), (6, 5)];
    let pick = sizes.iter().find(|(w, h)| (*h as u16 + 2) <= avail && (4 * *w as u16 + 9) <= inner.width);
    match pick {
        Some(&(w, h)) => {
            let y0 = top + (avail - (h as u16 + 2)) / 2;
            for (i, row) in big_time(&hhmm, w, h).iter().enumerate() {
                put(buf, inner, inner.x, y0 + i as u16, row, st(t.fg));
            }
            put(buf, inner, inner.x, y0 + h as u16 + 1, &osline, st(t.dim));
        }
        None if avail >= 2 => {
            put(buf, inner, inner.x, top + (avail - 2) / 2, &hhmm, st(t.fg).add_modifier(Modifier::BOLD));
            put(buf, inner, inner.x, top + (avail - 2) / 2 + 1, &osline, st(t.dim));
        }
        None => {}
    }

    let label_w = 8u16;
    let vx = inner.x + label_w;
    let mut cursor = None;
    for row in 0..ROWS {
        let y = fy + row as u16;
        let focused = app.focus == row && !popup;
        let label = ["login", "pass", "sess"][row];
        if focused {
            put(buf, area, inner.x - 2, y, "›", st(t.amber)); // in the gutter, so labels don't shift
        }
        put(buf, inner, inner.x, y, label, st(if focused { t.amber } else { t.dim }));
        let row_area = Rect { x: inner.x.saturating_sub(2), y, width: form_w + 2, height: 1 };
        match row {
            0 | 1 => {
                let input = if row == 0 { &app.user } else { &app.pass };
                let avail = (form_w - label_w).saturating_sub(if row == 1 { 6 } else { 0 }) as usize;
                let chars: Vec<char> = input.text.chars().collect();
                let start = if input.cursor >= avail { input.cursor + 1 - avail } else { 0 };
                hits.push(Hit { area: row_area, target: Target::Text { row, x0: vx, start } });
                let shown: String = chars
                    .iter()
                    .skip(start)
                    .take(avail)
                    .map(|&c| if row == 1 && !app.show_pass() { '*' } else { c })
                    .collect();
                if shown.is_empty() && row == 0 && !focused {
                    put(buf, inner, vx, y, "username", st(t.faint));
                } else {
                    put(buf, inner, vx, y, &shown, st(t.fg));
                }
                if focused && !app.busy {
                    cursor = Some((vx + (input.cursor - start) as u16, y));
                }
                if row == 1 && app.caps == Some(true) {
                    put(buf, inner, inner.x + form_w - 4, y, "CAPS", st(t.amber));
                }
            }
            _ => {
                let s = &app.sessions[app.si];
                let arrow = st(if focused { t.amber } else { t.dim });
                hits.push(Hit { area: row_area, target: Target::Sess });
                let mut x = put(buf, inner, vx, y, "‹ ", arrow);
                hits.push(Hit { area: Rect { x: vx, y, width: 2, height: 1 }, target: Target::SessPrev });
                x = put(buf, inner, x, y, &app.session_label(), st(t.fg));
                x = put(buf, inner, x + 1, y, s.tag(), st(t.dim));
                hits.push(Hit { area: Rect { x, y, width: 2, height: 1 }, target: Target::SessNext });
                put(buf, inner, x, y, " ›", arrow);
            }
        }
    }

    let my = fy + 4;
    let msg_style = st(match app.msg_kind {
        MsgKind::Dim => t.dim,
        MsgKind::Error => t.red,
        MsgKind::Busy => t.amber,
    });
    let mut msg = app.msg.clone();
    if let Some(step) = app.anim_step() {
        msg.push_str(&".".repeat((step % 4) as usize));
    }
    let msg_room = if keys_inline { form_w + 2 } else { inner.width };
    put(buf, Rect { width: msg_room, ..inner }, inner.x, my, &msg, msg_style);

    let (mut kx, ky) = if keys_inline { (inner.right() - keys_w, my) } else { (inner.x, my + 2) };
    for (i, (n, label)) in KEYS.iter().enumerate() {
        let label = if *n == 7 && app.reveal { "hide pass" } else { label };
        let focused = app.focus == ROWS + i && !popup;
        let lit = *n == 7 && app.show_pass();
        let key_style = if focused { st(t.amber).bg(t.faint) } else { st(t.fg) };
        let label_style = if focused { st(t.amber).bg(t.faint) } else if lit { st(t.amber) } else { st(t.dim) };
        let x0 = kx;
        kx = put(buf, inner, kx, ky, &format!("F{n}"), key_style);
        kx = put(buf, inner, kx, ky, &format!(" {label}"), label_style);
        hits.push(Hit { area: Rect { x: x0, y: ky, width: kx - x0, height: 1 }, target: Target::Key(i) });
        kx += 2;
    }

    if app.ipc_open {
        draw_ipc(buf, area, app, theme, hits);
    }
    match &app.overlay {
        Overlay::Help => draw_help(buf, area, app, theme, hits),
        Overlay::Menu(m) => {
            let rows: Vec<(String, String, bool)> = m.items.iter().map(|i| (i.label.clone(), i.desc.clone(), i.current)).collect();
            draw_menu(buf, area, m.title, &rows, m.index, theme, hits)
        }
        Overlay::None => {
            if let Some(c) = cursor {
                f.set_cursor_position(c);
            }
        }
    }
}

trait RenderRef {
    fn render_ref(self, area: Rect, buf: &mut Buffer);
}
impl RenderRef for Block<'_> {
    fn render_ref(self, area: Rect, buf: &mut Buffer) {
        ratatui::widgets::Widget::render(self, area, buf);
    }
}

fn boxed(buf: &mut Buffer, area: Rect, w: u16, h: u16, title: &str, t: Theme, hits: &mut Vec<Hit>) -> Rect {
    let w = w.min(area.width.saturating_sub(4));
    let h = h.min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    ratatui::widgets::Widget::render(Clear, r, buf);
    hits.push(Hit { area: r, target: Target::Popup });
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(st(t.dim))
        .style(Style::default().bg(t.panel).fg(t.fg))
        .title(Line::from(Span::styled(format!(" {title} "), st(t.amber))));
    let inner = block.inner(r);
    block.render_ref(r, buf);
    Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner }
}

fn draw_menu(buf: &mut Buffer, area: Rect, title: &str, rows: &[(String, String, bool)], index: usize, t: Theme, hits: &mut Vec<Hit>) {
    let desc_w = rows.iter().map(|r| width(&r.1)).max().unwrap_or(0);
    let label_w = rows.iter().map(|r| width(&r.0) + 2).max().unwrap_or(0);
    let footer = "↑↓ move · Enter select · Tab next menu · Esc close";
    let w = (label_w + desc_w + 10).max(width(footer) + 4).max(40);
    let inner = boxed(buf, area, w, rows.len() as u16 + 4, title, t, hits);
    // two rows below the items: a blank and the footer; scroll so the selection stays visible
    let visible = (inner.height as usize).saturating_sub(2);
    let first = (index + 1).saturating_sub(visible);
    for (y, (i, (label, desc, current))) in (inner.y..).zip(rows.iter().enumerate().skip(first).take(visible)) {
        let active = i == index;
        let bg = if active { t.faint } else { t.panel };
        let row = Rect { y, height: 1, ..inner };
        hits.push(Hit { area: row, target: Target::MenuItem(i) });
        buf.set_style(row.intersection(buf.area), Style::default().bg(bg));
        if active {
            put(buf, row, inner.x, y, "›", st(t.amber).bg(bg));
        }
        let mut x = put(buf, row, inner.x + 2, y, label, st(t.fg).bg(bg));
        if *current {
            x = put(buf, row, x, y, " *", st(t.dim).bg(bg));
        }
        let dx = inner.right().saturating_sub(width(desc)).max(x + 2);
        put(buf, row, dx, y, desc, st(t.dim).bg(bg));
    }
    let footer = if visible < rows.len() { format!("{}/{} · {footer}", index + 1, rows.len()) } else { footer.to_string() };
    put(buf, inner, inner.x, inner.bottom().saturating_sub(1), &footer, st(t.dim));
}

fn draw_help(buf: &mut Buffer, area: Rect, app: &App, t: Theme, hits: &mut Vec<Hit>) {
    let alt = if app.enhanced { "show password while held" } else { "show password while held (needs foot/kitty)" };
    let mut rows: Vec<(&str, &str)> = vec![
        ("Tab ↑ ↓", "move between fields and the key bar"),
        ("← →", "change session · move along the key bar"),
        ("Tab in a menu", "switch to the next menu"),
        ("Enter", "next field / sign in"),
        ("Esc", "cancel sign-in · clear password"),
        ("Ctrl+U / Ctrl+W", "clear field / delete word"),
        ("F2 F3 F4", "users · sessions · power"),
        ("F7", "show or hide password"),
        ("hold Alt", alt),
        ("F9", "messages exchanged with greetd"),
    ];
    if app.demo {
        rows.push(("demo", "any listed user, password: demo · Ctrl+C quits"));
    }
    let inner = boxed(buf, area, 64, rows.len() as u16 + 4, "help", t, hits);
    for (i, (k, d)) in rows.iter().enumerate() {
        let y = inner.y + i as u16;
        put(buf, inner, inner.x, y, k, st(t.fg));
        put(buf, inner, inner.x + 18, y, d, st(t.dim));
    }
    put(buf, inner, inner.x, inner.bottom().saturating_sub(1), "Tab next menu · Esc close", st(t.dim));
}

fn draw_ipc(buf: &mut Buffer, area: Rect, app: &App, t: Theme, hits: &mut Vec<Hit>) {
    let w = (area.width * 45 / 100).clamp(30, 72).min(area.width);
    let r = Rect { x: area.right() - w, width: w, ..area };
    ratatui::widgets::Widget::render(Clear, r, buf);
    hits.push(Hit { area: r, target: Target::Panel });
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(st(t.faint))
        .style(Style::default().bg(t.panel))
        .title(Line::from(Span::styled(" greetd IPC ", st(t.amber))));
    let inner = block.inner(r);
    block.render_ref(r, buf);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), y: inner.y + 1, height: inner.height.saturating_sub(1) };
    let n = inner.height as usize;
    let skip = app.ipc.len().saturating_sub(n);
    for (i, (time, dir, text)) in app.ipc.iter().skip(skip).enumerate() {
        let y = inner.y + i as u16;
        let c = match dir {
            '→' => t.amber,
            '←' => t.green,
            _ => t.dim,
        };
        let mut x = put(buf, inner, inner.x, y, time, st(t.dim));
        x = put(buf, inner, x + 1, y, &dir.to_string(), st(c));
        put(buf, inner, x + 1, y, text, st(if *dir == '$' { t.dim } else { t.fg }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_render() {
        let rows = big_time("20:38", 6, 5);
        for r in &rows {
            println!("{r}");
        }
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r.chars().count() == 4 * 6 + 9));
    }

    #[test]
    fn frame() {
        use crate::{app::App, system::*};
        use ratatui::{backend::TestBackend, Terminal};
        let (tx, _rx) = std::sync::mpsc::channel();
        let sys = SystemInfo { host: "Ayaan".into(), os: "Arch Linux".into(), kernel: "7.2.8-arch1-2".into(), tty: "tty1".into() };
        let users = vec![User { name: "ibra".into(), uid: 1000, shell: "/usr/bin/fish".into() }];
        let dir = std::env::temp_dir().join("mono-greeter-test");
        let app = App::new(sys, users, read_sessions(), dir, tx, true, true, None);
        let (w, h) = (124, 30);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, &app, Theme::truecolor(), &mut Vec::new())).unwrap();
        let buf = term.backend().buffer();
        for y in 0..h {
            let line: String = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
            println!("|{}|", line.trim_end());
        }
    }

    #[test]
    fn mouse() {
        use crate::app::{App, Overlay};
        use crate::system::*;
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::{backend::TestBackend, Terminal};

        let session = |id: &str, name: &str| Session {
            id: id.into(),
            name: name.into(),
            kind: SessionKind::Wayland,
            exec: vec![name.into()],
            desktop_names: String::new(),
            source: String::new(),
        };
        let (tx, _rx) = std::sync::mpsc::channel();
        let sys = SystemInfo { host: "box".into(), os: "Arch Linux".into(), kernel: "7".into(), tty: "tty1".into() };
        let users = vec![
            User { name: "alex".into(), uid: 1000, shell: "/usr/bin/fish".into() },
            User { name: "sam".into(), uid: 1001, shell: "/bin/bash".into() },
        ];
        let dir = std::env::temp_dir().join("mono-greeter-mouse-test");
        let mut app = App::new(sys, users, vec![session("a.desktop", "A"), session("b.desktop", "B")], dir, tx, true, true, None);
        let mut term = Terminal::new(TestBackend::new(124, 30)).unwrap();
        let mut hits = Vec::new();
        let mut redraw = |app: &App, hits: &mut Vec<Hit>| {
            term.draw(|f| draw(f, app, Theme::truecolor(), hits)).unwrap();
        };
        let at = |hits: &Vec<Hit>, t: Target| hits.iter().find(|h| h.target == t).map(|h| (h.area.x, h.area.y)).unwrap();
        let ev = |kind, (x, y): (u16, u16)| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
        let click = |p| ev(MouseEventKind::Down(MouseButton::Left), p);

        redraw(&app, &mut hits);
        assert!(!app.on_mouse(ev(MouseEventKind::Moved, (0, 0)), &hits), "plain moves don't redraw");

        assert!(app.on_mouse(click(at(&hits, Target::SessNext)), &hits));
        assert_eq!((app.si, app.focus), (1, 2), "› picks the next session and focuses sess");
        redraw(&app, &mut hits);
        app.on_mouse(ev(MouseEventKind::ScrollUp, at(&hits, Target::Sess)), &hits);
        assert_eq!(app.si, 0, "scrolling over sess goes back");

        app.on_mouse(click(at(&hits, Target::Key(1))), &hits);
        assert!(matches!(app.overlay, Overlay::Menu(_)), "F2 users opens the menu");
        redraw(&app, &mut hits);
        app.on_mouse(ev(MouseEventKind::Moved, at(&hits, Target::MenuItem(1))), &hits);
        assert!(matches!(&app.overlay, Overlay::Menu(m) if m.index == 1), "hover highlights");
        app.on_mouse(click(at(&hits, Target::MenuItem(1))), &hits);
        assert_eq!((app.user.text.as_str(), app.focus), ("sam", 1), "clicking an item picks it");

        redraw(&app, &mut hits);
        app.on_mouse(click(at(&hits, Target::Key(3))), &hits);
        redraw(&app, &mut hits);
        app.on_mouse(click((0, 0)), &hits);
        assert!(matches!(app.overlay, Overlay::None), "clicking outside closes the menu");

        redraw(&app, &mut hits);
        let (x0, y) = match hits.iter().find(|h| matches!(h.target, Target::Text { row: 0, .. })).unwrap().target {
            Target::Text { x0, .. } => (x0, at(&hits, Target::Sess).1 - 2),
            _ => unreachable!(),
        };
        app.on_mouse(click((x0 + 1, y)), &hits);
        assert_eq!((app.focus, app.user.cursor), (0, 1), "clicking in the text places the cursor");
    }

    fn test_app(users: usize, sessions: usize, tx: std::sync::mpsc::Sender<crate::greetd::Cmd>, backend_error: Option<String>) -> crate::app::App {
        use crate::system::*;
        let sys = SystemInfo { host: "box".into(), os: "Arch".into(), kernel: "7".into(), tty: "tty1".into() };
        let dir = std::env::temp_dir().join(format!("mono-greeter-test-{users}-{sessions}"));
        let users = (0..users).map(|i| User { name: format!("user{i}"), uid: 1000 + i as u32, shell: "/bin/sh".into() }).collect();
        let sessions = (0..sessions)
            .map(|i| Session {
                id: format!("s{i}.desktop"),
                name: format!("S{i}"),
                kind: SessionKind::Wayland,
                exec: vec![format!("s{i}")],
                desktop_names: String::new(),
                source: String::new(),
            })
            .collect();
        crate::app::App::new(sys, users, sessions, dir, tx, false, true, backend_error)
    }

    fn key(app: &mut crate::app::App, code: crossterm::event::KeyCode) {
        app.on_key(crossterm::event::KeyEvent::from(code));
    }

    fn screen(app: &crate::app::App) -> String {
        use ratatui::{backend::TestBackend, Terminal};
        let mut term = Terminal::new(TestBackend::new(124, 30)).unwrap();
        term.draw(|f| draw(f, app, Theme::truecolor(), &mut Vec::new())).unwrap();
        let buf = term.backend().buffer();
        (0..30).map(|y| (0..124).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect()
    }

    #[test]
    fn long_menus_scroll_instead_of_crashing() {
        use crossterm::event::KeyCode;
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = test_app(40, 2, tx, None);
        key(&mut app, KeyCode::F(2));
        assert!(screen(&app).contains("user0 "));
        key(&mut app, KeyCode::Up); // wraps to the last entry, "other…"
        let s = screen(&app);
        assert!(s.contains("other…") && s.contains("41/41"), "the selection scrolls into view:\n{s}");
        assert!(!s.contains("user0 "));
    }

    #[test]
    fn unreachable_greetd_unlocks_the_form() {
        use crate::app::MsgKind;
        use crossterm::event::KeyCode;
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        let mut app = test_app(1, 1, tx, Some("Can't reach greetd".into()));
        for c in "pw".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        assert!(!app.busy, "the form isn't left waiting on a dead greetd thread");
        assert_eq!((app.msg.as_str(), app.msg_kind == MsgKind::Error), ("Can't reach greetd", true));
        key(&mut app, KeyCode::Esc);
        assert!(!app.busy && app.msg != "Cancelling");
    }

    #[test]
    fn follow_up_prompt_fixes_user_and_session() {
        use crate::app::Overlay;
        use crate::greetd::Ev;
        use crossterm::event::KeyCode;
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = test_app(2, 2, tx, None);
        app.on_backend(Ev::Prompt { secret: true, text: "Verification code:".into() });
        let (user, si) = (app.user.text.clone(), app.si);
        key(&mut app, KeyCode::BackTab);
        key(&mut app, KeyCode::Char('x'));
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::F(2));
        key(&mut app, KeyCode::F(3));
        assert_eq!((app.user.text.clone(), app.si), (user, si));
        assert!(matches!(app.overlay, Overlay::None));
        key(&mut app, KeyCode::Esc);
        assert!(app.busy && app.msg == "Cancelling", "the form stays locked until greetd confirms");
    }
}
