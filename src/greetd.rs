use crate::ipc::{AuthMessageType, ErrorType, Request, Response};
use std::{
    os::unix::net::UnixStream,
    sync::mpsc::{Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

pub enum Cmd {
    Login { user: String, password: String, cmd: Vec<String>, env: Vec<String> },
    Reply(Option<String>),
    Cancel,
}

pub enum Ev {
    Log(char, String),
    Prompt { secret: bool, text: String },
    Info(String),
    PamError(String),
    AuthFailed,
    Failed(String),
    Started,
    Cancelled,
}

pub fn spawn_real(rx: Receiver<Cmd>, tx: Sender<Ev>) -> Result<(), String> {
    let path = std::env::var("GREETD_SOCK")
        .map_err(|_| "GREETD_SOCK is not set, so greetd isn't running this. Use --demo to try it.".to_string())?;
    let stream = UnixStream::connect(&path).map_err(|e| format!("Can't reach greetd at {path}: {e}"))?;
    thread::spawn(move || {
        let mut conn = Conn { stream, tx: tx.clone() };
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Login { user, password, cmd, env } => {
                    if let Err(e) = login(&mut conn, &rx, user, password, cmd, env) {
                        let _ = tx.send(Ev::Failed(e));
                    }
                }
                Cmd::Cancel => {
                    let _ = tx.send(Ev::Cancelled);
                }
                Cmd::Reply(_) => {}
            }
        }
    });
    Ok(())
}

struct Conn {
    stream: UnixStream,
    tx: Sender<Ev>,
}

impl Conn {
    fn send(&mut self, req: &Request) -> Result<(), String> {
        let _ = self.tx.send(Ev::Log('→', describe(req)));
        req.write_to(&mut self.stream).map_err(|e| format!("greetd: {e}"))
    }

    fn recv(&mut self) -> Result<Response, String> {
        let resp = Response::read_from(&mut self.stream).map_err(|e| format!("greetd: {e}"))?;
        let _ = self.tx.send(Ev::Log('←', serde_json::to_string(&resp).unwrap_or_default()));
        Ok(resp)
    }

    fn cancel(&mut self) -> Result<(), String> {
        self.send(&Request::CancelSession)?;
        let _ = self.recv();
        let _ = self.tx.send(Ev::Cancelled);
        Ok(())
    }
}

fn describe(req: &Request) -> String {
    match req {
        Request::PostAuthMessageResponse { response: Some(_) } => {
            serde_json::json!({ "type": "post_auth_message_response", "response": "***" }).to_string()
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn cancel_requested(rx: &Receiver<Cmd>) -> bool {
    let mut cancel = false;
    while let Ok(cmd) = rx.try_recv() {
        cancel |= matches!(cmd, Cmd::Cancel);
    }
    cancel
}

fn wait_reply(rx: &Receiver<Cmd>) -> Option<Option<String>> {
    loop {
        match rx.recv() {
            Ok(Cmd::Reply(r)) => return Some(r),
            Ok(Cmd::Cancel) | Err(_) => return None,
            Ok(Cmd::Login { .. }) => continue,
        }
    }
}

fn login(
    c: &mut Conn,
    rx: &Receiver<Cmd>,
    user: String,
    password: String,
    cmd: Vec<String>,
    env: Vec<String>,
) -> Result<(), String> {
    c.send(&Request::CreateSession { username: user })?;
    // The form's password answers the first secret prompt; any further prompt goes to the user.
    let mut password = Some(password);
    let mut starting = false;
    loop {
        let resp = c.recv()?;
        if cancel_requested(rx) {
            return c.cancel();
        }
        match resp {
            Response::AuthMessage { auth_message_type, auth_message } => match auth_message_type {
                AuthMessageType::Secret | AuthMessageType::Visible => {
                    let secret = matches!(auth_message_type, AuthMessageType::Secret);
                    let answer = match (secret, password.take()) {
                        (true, Some(p)) => Some(p),
                        (_, leftover) => {
                            password = leftover;
                            let _ = c.tx.send(Ev::Prompt { secret, text: auth_message });
                            match wait_reply(rx) {
                                Some(r) => r,
                                None => return c.cancel(),
                            }
                        }
                    };
                    c.send(&Request::PostAuthMessageResponse { response: answer })?;
                }
                AuthMessageType::Info => {
                    let _ = c.tx.send(Ev::Info(auth_message));
                    c.send(&Request::PostAuthMessageResponse { response: None })?;
                }
                AuthMessageType::Error => {
                    let _ = c.tx.send(Ev::PamError(auth_message));
                    c.send(&Request::PostAuthMessageResponse { response: None })?;
                }
            },
            Response::Success if !starting => {
                starting = true;
                c.send(&Request::StartSession { cmd: cmd.clone(), env: env.clone() })?;
            }
            Response::Success => {
                let _ = c.tx.send(Ev::Started);
                return Ok(());
            }
            Response::Error { error_type, description } => {
                c.send(&Request::CancelSession)?;
                let _ = c.recv();
                let ev = match error_type {
                    ErrorType::AuthError => Ev::AuthFailed,
                    ErrorType::Error => Ev::Failed(description),
                };
                let _ = c.tx.send(ev);
                return Ok(());
            }
        }
    }
}

pub fn spawn_demo(rx: Receiver<Cmd>, tx: Sender<Ev>, users: Vec<String>) {
    thread::spawn(move || {
        let log = |dir: char, s: String| {
            let _ = tx.send(Ev::Log(dir, s));
        };
        let req = |r: &Request| describe(r);
        let resp = |r: &Response| serde_json::to_string(r).unwrap_or_default();
        while let Ok(cmd) = rx.recv() {
            let Cmd::Login { user, password, cmd, env } = cmd else {
                let _ = tx.send(Ev::Cancelled);
                continue;
            };
            log('→', req(&Request::CreateSession { username: user.clone() }));
            thread::sleep(Duration::from_millis(150));
            log('←', resp(&Response::AuthMessage { auth_message_type: AuthMessageType::Secret, auth_message: "Password: ".into() }));
            log('→', req(&Request::PostAuthMessageResponse { response: Some(password.clone()) }));

            let ok = users.contains(&user) && password == "demo";
            let until = Instant::now() + Duration::from_millis(if ok { 600 } else { 2000 });
            let mut cancelled = false;
            while Instant::now() < until && !cancelled {
                thread::sleep(Duration::from_millis(50));
                cancelled = cancel_requested(&rx);
            }
            if cancelled {
                log('→', req(&Request::CancelSession));
                log('←', resp(&Response::Success));
                let _ = tx.send(Ev::Cancelled);
                continue;
            }
            if ok {
                log('←', resp(&Response::Success));
                log('→', req(&Request::StartSession { cmd, env }));
                log('←', resp(&Response::Success));
                let _ = tx.send(Ev::Started);
            } else {
                let desc = "pam_authenticate: AUTH_ERR".to_string();
                log('←', resp(&Response::Error { error_type: ErrorType::AuthError, description: desc.clone() }));
                log('→', req(&Request::CancelSession));
                log('←', resp(&Response::Success));
                let _ = tx.send(Ev::AuthFailed);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_hides_the_answer_and_its_length() {
        let line = describe(&Request::PostAuthMessageResponse { response: Some("hunter2".into()) });
        assert!(line.contains("\"***\"") && !line.contains("hunter2") && !line.contains("*******"));
        assert!(describe(&Request::PostAuthMessageResponse { response: None }).contains("null"));
    }
}
