use std::fs::File;
use std::io::ErrorKind::AlreadyExists;
use std::io::{BufRead, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bufstream::BufStream;

use crate::server::user::{Email, User};
use crate::server::Server;

// Just bail if there is some error.
// Used when performing operations on a TCP Stream generally
#[macro_export]
macro_rules! return_on_err(
    ($inp:expr) => {
        if $inp.is_err() {
            return;
        }
    }
);

macro_rules! delivery_ioerror(
    ($res:ident) => ({
        $res.push_str("451 Error in processing.\r\n");
        break;
    })
);

macro_rules! grab_email_token(
    ($arg:expr) => {
        match $arg {
            Some(from_path) => from_path.trim_start_matches('<').trim_end_matches('>'),
            _ => { return None; }
        }
    }
);

struct Lmtp<'a> {
    rev_path: Option<Email>,
    to_path: Vec<&'a User>,
    data: String,
    quit: bool,
}

static OK: &'static str = "250 OK\r\n";

impl<'a> Lmtp<'a> {
    fn deliver(&self) -> String {
        if self.to_path.is_empty() {
            return "503 Bad sequence - no recipients".to_string();
        }
        let mut res = String::new();
        for rcpt in &self.to_path {
            let mut timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(x) => x.as_secs(),
                Err(_) => {
                    res.push_str("555 UNIX time error\r\n");
                    break;
                }
            };
            let maildir = rcpt.maildir.clone();
            let newdir_path = Path::new(&maildir[..]).join("new");
            loop {
                let file_path = &newdir_path.join(timestamp.to_string());
                match File::create(&file_path) {
                    Err(e) => {
                        if e.kind() == AlreadyExists {
                            timestamp += 1;
                        } else {
                            warn!("Error creating file '{}': {}", file_path.to_str().unwrap_or_default(), e);
                            delivery_ioerror!(res);
                        }
                    }
                    Ok(mut file) => {
                        if file.write(self.data.as_bytes()).is_err() {
                            warn!("Error creating file '{}': cannot write file", file_path.to_str().unwrap_or_default());
                            delivery_ioerror!(res);
                        }
                        if file.flush().is_err() {
                            warn!("Error creating file '{}': cannot flush", file_path.to_str().unwrap_or_default());
                            delivery_ioerror!(res);
                        }
                        res.push_str("250 OK\r\n");
                        break;
                    }
                }
            }
        }
        res
    }
}

fn grab_email(arg: Option<&str>) -> Option<Email> {
    let from_path_split = match arg {
        Some(full_from_path) => {
            let mut split_arg = full_from_path.split(':');
            match split_arg.next() {
                Some(from_str) => match &from_str.to_ascii_lowercase()[..] {
                    "from" | "to" => {
                        grab_email_token!(split_arg.next())
                    }
                    _ => {
                        return None;
                    }
                },
                _ => {
                    return None;
                }
            }
        }
        _ => {
            return None;
        }
    };
    let mut from_parts = from_path_split.split('@');
    let local_part = match from_parts.next() {
        Some(part) => part.to_string(),
        _ => {
            return None;
        }
    };
    let domain_part = match from_parts.next() {
        Some(part) => part.to_string(),
        _ => {
            return None;
        }
    };
    Some(Email::new(local_part, domain_part))
}

pub fn serve(serv: Arc<Server>, mut stream: BufStream<TcpStream>) {
    let mut l = Lmtp {
        rev_path: None,
        to_path: Vec::new(),
        data: String::new(),
        quit: false,
    };
    return_on_err!(stream.write(format!("220 {} LMTP server ready\r\n", *serv.host()).as_bytes()));
    return_on_err!(stream.flush());
    loop {
        let mut command = String::new();
        match stream.read_line(&mut command) {
            Ok(_) => {
                if command.is_empty() {
                    return;
                }
                let trimmed_command = (&command[..]).trim();
                let mut args = trimmed_command.split(' ');
                let invalid = "500 Invalid command\r\n".to_string();
                let data_res = b"354 Start mail input; end with <CRLF>.<CRLF>\r\n";
                let ok_res = OK.to_string();
                let res = match args.next() {
                    Some(cmd) => {
                        warn!("LMTP Cmd: {}", trimmed_command);
                        match &cmd.to_ascii_lowercase()[..] {
                            "lhlo" => match args.next() {
                                Some(domain) => {
                                    format!("250 {}\r\n", domain)
                                }
                                _ => invalid,
                            },
                            "rset" => {
                                l.rev_path = None;
                                l.to_path = Vec::new();
                                ok_res
                            }
                            "noop" => ok_res,
                            "quit" => {
                                l.quit = true;
                                format!("221 {} Closing connection\r\n", *serv.host())
                            }
                            "vrfy" => invalid,
                            "mail" => match grab_email(args.next()) {
                                None => invalid,
                                s => {
                                    l.rev_path = s;
                                    ok_res
                                }
                            },
                            "rcpt" => match l.rev_path {
                                None => invalid,
                                _ => match grab_email(args.next()) {
                                    None => invalid,
                                    Some(email) => match serv.users.get(&email) {
                                        None => format!("550 No such user {}\r\n", email),
                                        Some(user) => {
                                            l.to_path.push(user);
                                            ok_res
                                        }
                                    },
                                },
                            },
                            "data" => {
                                return_on_err!(stream.write(data_res));
                                return_on_err!(stream.flush());
                                let mut loop_res = invalid;
                                loop {
                                    let mut data_command = String::new();
                                    match stream.read_line(&mut data_command) {
                                        Ok(_) => {
                                            if data_command.is_empty() {
                                                break;
                                            }
                                            let data_cmd = (&data_command[..]).trim();
                                            if data_cmd == "." {
                                                loop_res = l.deliver();
                                                l.data = String::new();
                                                break;
                                            }
                                            l.data.push_str(data_cmd);
                                            l.data.push('\n');
                                        }
                                        _ => {
                                            break;
                                        }
                                    }
                                }
                                loop_res
                            }
                            _ => invalid,
                        }
                    }
                    None => invalid,
                };
                return_on_err!(stream.write(res.as_bytes()));
                return_on_err!(stream.flush());
                if l.quit {
                    return;
                }
            }
            _ => {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grab_email_to() {
        let email = Some("to:<user@example.com>");
        let parsed_email = grab_email(email);
        assert!(parsed_email.is_some());
        let userhost = parsed_email.unwrap();
        assert_eq!(userhost.local_part, "user");
        assert_eq!(userhost.domain_part, "example.com");
    }

    #[test]
    fn test_grab_email_from() {
        let email = Some("from:<user1@example.com>");
        let parsed_email = grab_email(email);
        assert!(parsed_email.is_some());
        let userhost = parsed_email.unwrap();
        assert_eq!(userhost.local_part, "user1");
        assert_eq!(userhost.domain_part, "example.com");
    }

    #[test]
    fn test_grab_email_raw_email_failure() {
        let email = Some("user1@example.com");
        let parsed_email = grab_email(email);
        assert!(parsed_email.is_none());
    }
}