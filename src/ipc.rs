use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession { username: String },
    PostAuthMessageResponse { response: Option<String> },
    StartSession {
        cmd: Vec<String>,
        #[serde(default)]
        env: Vec<String>,
    },
    CancelSession,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error { error_type: ErrorType, description: String },
    AuthMessage { auth_message_type: AuthMessageType, auth_message: String },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    AuthError,
    Error,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum AuthMessageType {
    Visible,
    Secret,
    Info,
    Error,
}

const MAX_MESSAGE: usize = 64 * 1024;

// greetd-ipc(7): a native-endian u32 length, then that many bytes of JSON.
fn write_message<T: Serialize>(stream: &mut impl Write, msg: &T) -> io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    let len = u32::try_from(body.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "message too large"))?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&len.to_ne_bytes());
    frame.extend_from_slice(&body);
    stream.write_all(&frame)
}

fn read_message<T: for<'de> Deserialize<'de>>(stream: &mut impl Read) -> io::Result<T> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("message of {len} bytes is too large")));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

impl Request {
    pub fn write_to(&self, stream: &mut impl Write) -> io::Result<()> {
        write_message(stream, self)
    }
}

impl Response {
    pub fn read_from(stream: &mut impl Read) -> io::Result<Self> {
        read_message(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wire(req: &Request) -> serde_json::Value {
        serde_json::to_value(req).unwrap()
    }

    #[test]
    fn request_json_matches_protocol() {
        assert_eq!(wire(&Request::CreateSession { username: "ibra".into() }), json!({"type": "create_session", "username": "ibra"}));
        assert_eq!(
            wire(&Request::PostAuthMessageResponse { response: Some("pw".into()) }),
            json!({"type": "post_auth_message_response", "response": "pw"})
        );
        assert_eq!(wire(&Request::PostAuthMessageResponse { response: None }), json!({"type": "post_auth_message_response", "response": null}));
        assert_eq!(
            wire(&Request::StartSession { cmd: vec!["fish".into(), "-l".into()], env: vec!["XDG_SESSION_TYPE=tty".into()] }),
            json!({"type": "start_session", "cmd": ["fish", "-l"], "env": ["XDG_SESSION_TYPE=tty"]})
        );
        assert_eq!(wire(&Request::CancelSession), json!({"type": "cancel_session"}));
    }

    #[test]
    fn responses_parse() {
        let parse = |s: &str| serde_json::from_str::<Response>(s).unwrap();
        assert_eq!(parse(r#"{"type":"success"}"#), Response::Success);
        assert_eq!(
            parse(r#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password: "}"#),
            Response::AuthMessage { auth_message_type: AuthMessageType::Secret, auth_message: "Password: ".into() }
        );
        assert_eq!(
            parse(r#"{"type":"error","error_type":"auth_error","description":"pam_authenticate: AUTH_ERR"}"#),
            Response::Error { error_type: ErrorType::AuthError, description: "pam_authenticate: AUTH_ERR".into() }
        );
    }

    #[test]
    fn framing_round_trip() {
        let mut buf = Vec::new();
        Request::CancelSession.write_to(&mut buf).unwrap();
        let body = br#"{"type":"cancel_session"}"#;
        assert_eq!(&buf[..4], &(body.len() as u32).to_ne_bytes());
        assert_eq!(&buf[4..], body);

        let reply = br#"{"type":"success"}"#;
        let mut framed = (reply.len() as u32).to_ne_bytes().to_vec();
        framed.extend_from_slice(reply);
        assert_eq!(Response::read_from(&mut framed.as_slice()).unwrap(), Response::Success);
    }

    #[test]
    fn rejects_oversized_frames() {
        let framed = (10_000_000u32).to_ne_bytes();
        assert!(Response::read_from(&mut framed.as_slice()).is_err());
    }
}
