use std::os::unix::net::UnixStream;
use std::net::Shutdown;
use std::error::Error;
use crate::utils::*;
use crate::utils::{LogLevel, LOG};
use crate::{ERR, INFO, WARN};
use std::vec::Vec;
pub struct Topic<'a,'b>
{
    name: &'a str,
    socket_file: &'b str,
    channel: Option<UnixStream>,
}

pub struct Msg
{
    kind: u8,
    channel_id: u16,
    client_id: u16,
    size: u32,
    data:Vec<u8>,
}

impl<'a,'b> Topic<'a,'b>
{
    /// Create new `Topic` object
    ///
    /// Arguments
    ///
    /// * `name` - a topic name
    /// * `socket_file` - a a path to tunnel socket
    pub fn create(name: &'a str, socket_file: &'b str) -> Self
    {
        Topic { name, socket_file, channel: None }
    }

    /// Open a tunnel for the topic
    ///
    pub fn open(&mut self) -> Result<(), Box<dyn Error>>
    {
        INFO!("Open unix domain socket: {}", self.socket_file);
        let sock = UnixStream::connect(self.socket_file)?;
        self.channel = Some(sock);
        Ok(())
    }

    /// Close the tunnel
    ///
    pub fn close(& self) -> Result<(), Box<dyn Error>>
    {
        // TODO: send close message
        INFO!("Closing the channel: {}", self.name);
        self.channel.as_ref()
            .ok_or("Channel is not created")?
            .shutdown(Shutdown::Both)?;
        Ok(())
    }
}


impl Msg
{

}