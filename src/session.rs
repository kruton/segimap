use std::io::TcpStream;
use std::io::{Buffer, BufferedStream};
use std::str::StrSlice;
use std::ascii::OwnedAsciiExt;
use std::sync::Arc;

use error::{Error,ImapStateError};
use login::LoginData;

pub use folder::Folder;
pub use server::Server;
use parser::fetch;

macro_rules! return_on_err(
    ($inp:expr) => {
        match $inp {
            Err(_) => { return; }
            _ => {}
        }
    }
)

static GREET: &'static [u8] = b"* OK Server ready.\r\n";

pub struct Session {
    stream: BufferedStream<TcpStream>,
    serv: Arc<Server>,
    logout: bool,
    maildir: Option<String>,
    folder: Option<Folder>
}

impl Session {
    pub fn new(stream: BufferedStream<TcpStream>, serv: Arc<Server>) -> Session {
        Session {
            stream: stream,
            serv: serv,
            logout: false,
            maildir: None,
            folder: None
        }
    }

    pub fn handle(&mut self) {
        return_on_err!(self.stream.write(GREET));
        return_on_err!(self.stream.flush());
        loop {
            match self.stream.read_line() {
                Ok(command) => {
                    if command.len() == 0 {
                        return;
                    }
                    let res = self.interpret(command.as_slice());
                    warn!("Response:\n{}", res);
                    return_on_err!(self.stream.write(res.as_bytes()));
                    return_on_err!(self.stream.flush());
                    if self.logout {
                        return;
                    }
                }
                Err(_) => { return; }
            }
        }
    }

    fn interpret(&mut self, command: &str) -> String {
        let mut args = command.trim().split(' ');
        let tag = args.next().unwrap();
        let bad_res = format!("{} BAD Invalid command\r\n", tag);
        match args.next() {
            Some(cmd) => {
                warn!("Cmd: {}", command.trim());
                match cmd.to_string().into_ascii_lower().as_slice() {
                    "capability" => {
                        return format!("* CAPABILITY IMAP4rev1\n{} OK Capability successful\n", tag);
                    }
                    "login" => {
                        let login_args: Vec<&str> = args.collect();
                        if login_args.len() < 2 { return bad_res; }
                        let email = login_args[0].trim_chars('"');
                        let password = login_args[1].trim_chars('"');
                        let no_res  = format!("{} NO invalid username or password\r\n", tag);
                        match LoginData::new(email.to_string(), password.to_string()) {
                            Some(login_data) => {
                                self.maildir = match self.serv.users.find(&login_data.email) {
                                    Some(user) => {
                                        if user.auth_data.verify_auth(login_data.password) {
                                            Some(user.maildir.clone())
                                        } else {
                                            None
                                        }
                                    }
                                    None => None
                                }
                            }
                            None => { return no_res; }
                        }
                        match self.maildir {
                            Some(_) => {
                                return format!("{} OK logged in successfully as {}\r\n", tag, email);
                            }
                            None => { return no_res; }
                        }
                    }
                    "logout" => {
                        self.logout = true;
                        match self.folder {
                            Some(ref folder) => {
                                folder.close();
                            }
                            _ => {}
                        }
                        return format!("* BYE Server logging out\r\n{} OK Server logged out\r\n", tag);
                    }
                    "select" => {
                        let select_args: Vec<&str> = args.collect();
                        if select_args.len() < 1 { return bad_res; }
                        let mailbox_name = select_args[0].trim_chars('"'); // "
                        match self.maildir {
                            None => { return bad_res; }
                            _ => {}
                        }
                        self.folder = self.select_mailbox(mailbox_name);
                        match self.folder {
                            None => {
                                return format!("{} NO error finding mailbox\n", tag);
                            }
                            Some(ref folder) => {
                                // * Flags
                                // Should match values in enum Flag in message.rs and \\Deleted
                                let mut ok_res = format!("* FLAGS (\\Answered \\Deleted \\Draft \\Flagged \\Seen)\n");
                                // * <n> EXISTS
                                ok_res = format!("{}* {} EXISTS\n", ok_res, folder.exists);
                                // * <n> RECENT
                                ok_res = format!("{}* {} RECENT\n", ok_res, folder.recent());
                                // * OK UNSEEN
                                ok_res = format!("{}* OK UNSEEN {}\n", ok_res, folder.unseen);
                                // * OK PERMANENTFLAG
                                // Should match values in enum Flag in message.rs and \\Deleted
                                ok_res = format!("{}* PERMANENTFLAGS (\\Answered \\Deleted \\Draft \\Flagged \\Seen) Permanent flags\n", ok_res);
                                // * OK UIDNEXT
                                // * OK UIDVALIDITY
                                return format!("{}{} OK [READ-WRITE] SELECT command was successful\n", ok_res, tag);
                            }
                        }
                    }
                    // examine
                    "create" => {
                        let create_args: Vec<&str> = args.collect();
                        if create_args.len() < 1 { return bad_res; }
                        let mailbox_name = create_args[0].trim_chars('"'); // "
                        // match magic_mailbox_create(mailbox_name.to_string()) {
                        //     Ok(_) => {
                        //         return format!("{} OK create completed", tag);
                        //     }
                        //     Err(_) => {
                        //         return format!("{} OK create failed", tag);
                        //     }
                        // }
                        return format!("{} OK unimplemented\n", tag);
                    }
                    // rename
                    "delete" => {
                        let delete_args: Vec<&str> = args.collect();
                        if delete_args.len() < 1 { return bad_res; }
                        let mailbox_name = delete_args[0].trim_chars('"'); // "
                        // match magic_mailbox_delete(mailbox_name.to_string()) {
                        //     Ok(_) => {
                        //         return format!("{} OK delete completed", tag);
                        //     }
                        //     Err(_) => {
                        //         return format!("{} OK delete failed", tag);
                        //     }
                        // }
                        return format!("{} OK unimplemented\n", tag);
                    }
                    "list" => {
                        let list_args: Vec<&str> = args.collect();
                        if list_args.len() < 2 { return bad_res; }
                        let reference = list_args[0].trim_chars('"');
                        let mailbox_name = list_args[1].trim_chars('"');
                        if mailbox_name.len() == 0 {
                            return format!("* LIST (\\Noselect) \"/\" \"{}\"\n{} OK List successful\n", reference, tag);
                        }
                        return format!("OK unimplemented\n");
                    }
                    "close" => {
                        return match self.close() {
                            Err(_) => bad_res,
                            Ok(_) => format!("{} OK close completed\n", tag)
                        }
                    }
                    "expunge" => {
                        match self.close() {
                            Err(_) => { return bad_res; }
                            Ok(v) => {
                                let mut ok_res = String::new();
                                for i in v.iter() {
                                    ok_res = format!("{}* {} EXPUNGE\n", ok_res, i);
                                }
                                return format!("{}{} OK expunge completed", ok_res, tag);
                            }
                        }
                    }
                    "fetch" => {
                        // Split the index prefix from the command.
                        let cmd: Vec<&str> = command.splitn(1, ' ').skip(1).collect();
                        // Remove the newline from the command.
                        let cmd = cmd[0].lines().next().unwrap();
                        // Remove the carriage return from the command.
                        let cmd: Vec<&str> = cmd.splitn(1, '\r').take(1).collect();
                        let cmd = cmd[0];
                        // Parse the command with the PEG parser.
                        let parsed_cmd = match fetch(cmd) {
                            Ok(cmd) => cmd,
                            _ => return bad_res
                        };
                        match self.folder {
                            None => { return bad_res; }
                            _ => {}
                        }
                        println!("CMD: {}", parsed_cmd);
                        return format!("{} OK unimplemented\n", tag);
                    }
                    _ => { return bad_res; }
                }
            }
            None => {}
        }
        bad_res
    }

    // should generate list of sequence numbers that were deleted
    fn close(&self) -> Result<Vec<uint>, Error> {
        match self.folder {
            None => { Err(Error::simple(ImapStateError, "Not in selected state")) }
            Some(ref folder) => {
                Ok(folder.close())
            }
        }
    }

    pub fn select_mailbox(&self, mailbox_name: &str) -> Option<Folder> {
        let mbox_name = regex!("INBOX").replace(mailbox_name, ".");
        match self.maildir {
            None => { None }
            Some(ref maildir) => {
                let maildir_path = Path::new(maildir.as_slice()).join(mbox_name);
                // TODO: recursively grab parent...
                Folder::new(mailbox_name.to_string(), None, maildir_path)
                    // TODO: Insert new folder into folder service
                    // folder_service.insert(mailbox_name.to_string(), box *folder);
            }
        }
    }
}
