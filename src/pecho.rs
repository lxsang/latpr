//! # //! Echo publisher example of the tunnel API
//!
//! **Author**: "Dany LE"
//!
use latpr::tunnel::{CallbackEvent, IOInterest, Msg, MsgKind, Topic};
use latpr::utils::*;
use latpr::utils::{LogLevel, LOG};
use latpr::{ERROR, EXIT, INFO, WARN};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::panic;
use std::vec::Vec;

/// Callback: clean up function
///
/// This function remove the unix socket file if
/// exist before quiting the program
///
/// # Arguments
///
/// * `n` - system exit code
fn clean_up(n: i32) {
    if n != 0 {
        panic!("{}", format!("pecho is terminated by system signal: {}", n));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // init the system log
    // Create an empty log object and keep it alive in the scope
    // of `main`. When this object is dropped, the syslog will
    // be closed automatically
    let _log = LOG::init_log();
    on_exit(clean_up);

    // read all the arguments
    let args: Vec<String> = env::args().collect();
    // there must be minimum 3 arguments:
    // - the program
    // - the socket file
    // - the topic name
    if args.len() != 4 {
        EXIT!("Invalid arguments: {}", format!("{:?}", args));
    }
    let _ = fs::remove_file(&args[3]);
    let socket = UnixDatagram::bind(&args[3])?;
    fs::set_permissions(&args[3], fs::Permissions::from_mode(0o777))?;
    let mut clients = HashMap::<u16, u16>::new();
    let mut msg_handle = |evt: &CallbackEvent, topic: &mut Topic| {
        if let Some(msg) = evt.msg {
            match msg.kind {
                MsgKind::ChannelSubscribe => {
                    INFO!("Client {} subscribe to channel {}", msg.client_id, &args[2]);
                    let _ = clients.insert(msg.client_id, msg.client_id);
                }
                MsgKind::ChannelUnsubscribe => {
                    INFO!(
                        "Client {} unsubscribe to channel {}",
                        msg.client_id,
                        &args[2]
                    );
                    if let None = clients.remove(&msg.client_id) {
                        WARN!("Client {} is not in the client list", msg.client_id);
                    }
                }
                MsgKind::ChannelUnsubscribeAll => {
                    for (key, _) in clients.iter() {
                        let msg = Msg::create(MsgKind::ChannelUnsubscribe, 0, *key, Vec::new());
                        topic.write(&msg)?;
                    }
                }
                _ => {
                    WARN!(
                        "Recive mesage kind {} from client {}",
                        msg.kind,
                        msg.client_id
                    );
                }
            };
        }
        let event = match evt.event {
            None => return Ok(()),
            Some(e) => e,
        };
        let _ = match evt.fd {
            None => return Ok(()),
            Some(d) => d,
        };
        if event.is_readable() {
            let mut buf = [0; 2048];
            let (count, _) = socket.recv_from(&mut buf)?;
            for (key, _) in clients.iter() {
                let msg = Msg::create(MsgKind::ChannelData, 0, *key, (&buf[0..count]).to_vec());
                topic.write(&msg)?;
            }
        }
        Ok(())
    };
    {
        let mut topic = Topic::create(&args[2], &args[1]);
        let mut running = true;
        topic.on_message(&mut msg_handle);
        topic.register_io(socket.as_raw_fd(), IOInterest::READABLE)?;
        topic.open()?;
        while running {
            if let Err(error) = topic.step() {
                ERROR!("Error step: {}", error);
                running = false;
            }
        }
    }
    Ok(())
}
