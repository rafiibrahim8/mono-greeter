mod app;
mod greetd;
mod ipc;
mod system;
mod ui;

use app::{App, Exit};
use crossterm::{
    cursor::SetCursorStyle,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

const USAGE: &str = "mono-greeter: a keyboard-first greeter for greetd

usage: mono-greeter [--demo] [--cache-dir DIR]

  --demo           run without greetd; listed users sign in with password \"demo\"
  --cache-dir DIR  where the last user and sessions are remembered
                   (default /var/cache/mono-greeter, or ~/.cache/mono-greeter with --demo)
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return;
    }
    let demo = args.iter().any(|a| a == "--demo");
    let cache_dir = args
        .iter()
        .position(|a| a == "--cache-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if demo {
                let base = std::env::var("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
                base.join("mono-greeter")
            } else {
                PathBuf::from("/var/cache/mono-greeter")
            }
        });

    let users = system::read_users();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let backend_error = if demo {
        greetd::spawn_demo(cmd_rx, ev_tx, users.iter().map(|u| u.name.clone()).collect());
        None
    } else {
        greetd::spawn_real(cmd_rx, ev_tx).err()
    };

    let result = run(
        |enhanced| {
            App::new(system::SystemInfo::read(), users, system::read_sessions(), cache_dir, cmd_tx, demo, enhanced, backend_error)
        },
        ev_rx,
    );

    match result {
        Ok(Some(Exit::Started(cmd))) if demo => println!("demo: greetd would now start: {}", cmd.join(" ")),
        Ok(_) => {}
        Err(e) => {
            eprintln!("mono-greeter: {e}");
            std::process::exit(1);
        }
    }
}

fn restore() {
    let mut out = io::stdout();
    // OSC 110/111/112: reset foreground, background and cursor colour
    let _ = write!(out, "\x1b]110\x1b\\\x1b]111\x1b\\\x1b]112\x1b\\");
    let _ = execute!(
        out,
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}

// OSC 10/11/12: foreground, background (also fills the padding around the grid), cursor colour
fn set_terminal_colors(out: &mut impl Write) -> io::Result<()> {
    if std::env::var("TERM").is_ok_and(|t| t == "linux") {
        return Ok(());
    }
    write!(out, "\x1b]10;#cdd2cd\x1b\\\x1b]11;#101211\x1b\\\x1b]12;#e8c07d\x1b\\")?;
    out.flush()
}

fn run(make_app: impl FnOnce(bool) -> App, ev_rx: mpsc::Receiver<greetd::Ev>) -> io::Result<Option<Exit>> {
    terminal::enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture, SetCursorStyle::BlinkingBlock)?;
    set_terminal_colors(&mut out)?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));

    let enhanced = matches!(terminal::supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        )?;
    }

    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;
    terminal.clear()?;
    let theme = ui::Theme::pick();
    let mut app = make_app(enhanced);

    let mut dirty = true;
    let mut hits = Vec::new();
    let mut shown = app.time_state();
    let result = loop {
        while let Ok(ev) = ev_rx.try_recv() {
            app.on_backend(ev);
            dirty = true;
        }
        let now = app.time_state();
        if now != shown {
            shown = now;
            dirty = true;
        }
        if dirty {
            if let Err(e) = terminal.draw(|f| ui::draw(f, &app, theme, &mut hits)) {
                break Err(e);
            }
            dirty = false;
        }
        if app.exit.is_some() {
            if matches!(app.exit, Some(Exit::Started(_))) {
                std::thread::sleep(Duration::from_millis(400)); // let "Starting …" show
            }
            break Ok(app.exit.take());
        }
        // greetd events are only read between polls, so never wait long
        let wait = app.until_next_change().clamp(Duration::from_millis(10), Duration::from_secs(1));
        match event::poll(wait) {
            Ok(true) => match event::read() {
                Ok(Event::Key(k)) => {
                    app.on_key(k);
                    dirty = true;
                }
                Ok(Event::Mouse(m)) => dirty |= app.on_mouse(m, &hits),
                Ok(Event::Resize(..)) => dirty = true,
                Ok(_) => {}
                Err(e) => break Err(e),
            },
            Ok(false) => {}
            Err(e) => break Err(e),
        }
    };
    restore();
    result
}
