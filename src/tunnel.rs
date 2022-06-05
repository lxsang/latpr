use std::os::unix::net::UnixStream;
use std::net::Shutdown;
use std::error::Error;
use std::io::{Read,Write};
use std::os::unix::io::{RawFd,AsRawFd};
use crate::utils::{LogLevel, LOG};
use crate::{ERR, INFO, WARN, EXIT, ERROR};
use std::vec::Vec;
use std::collections::HashMap;
use mio::{Events, Interest, Poll, Token};
use mio::event::Event;
use mio::unix::SourceFd;
use std::time::Duration;

const MSG_MAGIC_BEGIN: u16 = 0x414e;
const MSG_MAGIC_END: u16 = 0x5444;
const SERVER: Token = Token(0);

pub type IOInterest = Interest;
pub type IOEvent = Event;
//pub type MsgCallback = dyn Fn(&Msg) -> Option<Msg>;
//pub type IoCallback = dyn Fn(&RawFd, &IOEvent) -> Option<Msg>;
/// Different message  type
///
pub enum MsgKind {
    /// OK
    ChannelOk,
    /// Error header
    ChannelError,
    /// Open a channel
    ChannelOpen,
    // Close channel
    ChannelClose,
    // Data
    ChannelData,
    // Unsubscribe a channel
    ChannelUnsubscribe,
    // Subscribe to a channel
    ChannelSubscribe,
    // CTRL
    ChannelCtrl,
    // Unknown Msg type,
    ChannelUnsubscribeAll,
    Unknown
}

pub struct CallbackEvent<'c>
{
    pub fd: Option<&'c RawFd>,
    pub event: Option<&'c IOEvent>,
    pub msg: Option<&'c Msg>,
}

pub struct Topic<'a>
{
    pub name: &'a str,
    pub socket_file: &'a str,
    channel: Option<UnixStream>,
    poll: Option<Poll>,
    msg_handle: Option<&'a mut dyn FnMut(&CallbackEvent) -> Option<Vec<Msg>>>,
    io_fds: HashMap<Token,RawFd>,
    stepto: Option<Duration>,
    n_token: usize,
}

pub struct Msg
{
    pub kind: MsgKind,
    pub channel_id: u16,
    pub client_id: u16,
    pub size: u32,
    pub data:Vec<u8>,
}

impl<'b> CallbackEvent<'b> {
    pub fn create(fd: Option<&'b RawFd>, event: Option<&'b IOEvent>, msg: Option<&'b Msg>) -> Self
    {
        CallbackEvent {fd,  event, msg}
    }
}

impl MsgKind {
    /// convert a u8 value to `MsgKind` value
    ///
    /// # Arguments
    ///
    /// * `value` - u8 header value
    fn from_u8(value: u8) -> Self {
        match value {
            0x0 => MsgKind::ChannelOk,
            0x1 => MsgKind::ChannelError,
            0x2 => MsgKind::ChannelSubscribe,
            0x3 => MsgKind::ChannelUnsubscribe,
            0x4 => MsgKind::ChannelOpen,
            0x5 => MsgKind::ChannelClose,
            0x6 => MsgKind::ChannelData,
            0x7 => MsgKind::ChannelCtrl,
            0xA => MsgKind::ChannelUnsubscribeAll,
            _ => MsgKind::Unknown,
        }
    }

    /// convert a `MsgKind` value to u8 value 
    ///
    /// # Arguments
    ///
    /// * `kind` - MsgKind
    fn to_u8(kind: & Self) -> u8 {
        match kind {
            MsgKind::ChannelOk => 0x0,
            MsgKind::ChannelError => 0x1,
            MsgKind::ChannelSubscribe => 0x2,
            MsgKind::ChannelUnsubscribe => 0x3,
            MsgKind::ChannelOpen => 0x4,
            MsgKind::ChannelClose => 0x5,
            MsgKind::ChannelData => 0x6,
            MsgKind::ChannelCtrl => 0x7,
            MsgKind::ChannelUnsubscribeAll => 0xA,
            MsgKind::Unknown => 0xFF,
        }
    }
}

impl<'a> Topic<'a>
{
    /// Create new `Topic` object
    ///
    /// Arguments
    ///
    /// * `name` - a topic name
    /// * `socket_file` - a a path to tunnel socket
    pub fn create(name: &'a str, socket_file: &'a str) -> Self
    {
        Topic { 
            name,
            socket_file,
            channel: None,
            poll: None,
            msg_handle: None,
            io_fds: HashMap::new(),
            stepto: None,
            n_token: 1,
        }
    }

    /// Open a tunnel for the topic
    ///
    pub fn open(&mut self) -> Result<(), Box<dyn Error>>
    {
        INFO!("Open unix domain socket: {}", self.socket_file);
        let sock = UnixStream::connect(self.socket_file)?;
        let fd = sock.as_raw_fd();
        self.channel = Some(sock);
        // send a channel open
        let rq = Msg::create(MsgKind::ChannelOpen, 0, 0, self.name.as_bytes().to_vec());
        self.write(&rq)?;
        // wait for confirm
        INFO!("Wait for comfirm channel opening from: {}", self.socket_file);
        let response = self.read()?;
        match response.kind
        {
            MsgKind::ChannelOk => {} ,
            _ => {
                let _ = self.close();
                EXIT!("Channel is not created: %s. Tunnel service responds with msg of type {}", response.kind);
            }
        }
        // add socket to polling
        let poll = self.get_poll()?;
        poll
            .registry()
            .register(&mut SourceFd(&fd), SERVER, Interest::READABLE)?;
        let _ = self.io_fds.insert(SERVER, fd);
        INFO!("Channel {} opened sucessfully", self.name);
        Ok(())
    }

    /// Read and check the number if any
    ///
    /// Arguments
    ///
    /// * `number` - the number to check, 0 if not check
    fn read_u16_number(&self, number: u16) -> Result<u16, Box<dyn Error>>
    {
        let mut buf:[u8;2] = [0;2];
        self.channel.as_ref()
            .ok_or("Invalid read channel")?
            .read_exact(&mut buf)?;
        let retnum = u16::from_be_bytes(buf);
        if (number != 0) && (retnum != number)
        {
            return Err(ERR!(format!("Read number mismatched, expected {:#04x} got {:#04x}", number, retnum)));
        }
        Ok(retnum)
    }

    /// Read u32 number
    ///
    fn read_u32_number(&self) -> Result<u32, Box<dyn Error>>
    {
        let mut buf:[u8;4] = [0;4];
        self.channel.as_ref()
            .ok_or("Invalid read channel")?
            .read_exact(&mut buf)?;
        Ok(u32::from_be_bytes(buf))
    }
    /// Read message type
    ///
    fn read_kind(&self) -> Result<MsgKind, Box<dyn Error>>
    {
        let mut buf:[u8;1] = [0];
        self.channel.as_ref()
            .ok_or("Invalid read channel")?
            .read_exact(&mut buf)?;
        if buf[0] > 0x7
       {
           return Err(ERR!(format!("Invalid msg type {:#02x}", buf[0])));
       } 
        Ok(MsgKind::from_u8(buf[0]))
    }

    /// Read a message from the socket
    ///
    fn read(&self) -> Result<Msg, Box<dyn Error>>
    {
        let _ = self.read_u16_number(MSG_MAGIC_BEGIN)?;
        let kind: MsgKind = self.read_kind()?;
        let channel_id: u16 = self.read_u16_number(0)?;
        let client_id: u16 = self.read_u16_number(0)?;
        let size: u32 = self.read_u32_number()?;
        let mut payload =  vec![0; size as usize];
        // read all the payload data
        self.channel.as_ref()
            .ok_or("Invalid read channel")?
            .read_exact(&mut payload)?;
        
        let _ = self.read_u16_number(MSG_MAGIC_END)?;

        let msg = Msg::create(kind, channel_id, client_id, payload);
        Ok(msg)
    }

    /// Write a message to the socket
    ///
    /// Arguments
    /// 
    /// * `msg` - a message
    fn write(&self, msg: &Msg) -> Result<(), Box<dyn Error>>
    {
        let mut sock = self.channel.as_ref()
            .ok_or("Invalid write channel")?;
        // write the magic begin
        sock.write_all(&MSG_MAGIC_BEGIN.to_be_bytes())?;
        sock.write_all(&[MsgKind::to_u8(& msg.kind)])?;
        sock.write_all(&msg.channel_id.to_be_bytes())?;
        sock.write_all(&msg.client_id.to_be_bytes())?;
        sock.write_all(&msg.size.to_be_bytes())?;
        if msg.size != 0
        {
            sock.write_all(&msg.data)?;
        }
        sock.write_all(&MSG_MAGIC_END.to_be_bytes())?;
        Ok(())
    }

    /// Close the tunnel
    ///
    fn close(& self) -> Result<(), Box<dyn Error>>
    {
        INFO!("Closing the channel: {}", self.name);
        let rq = Msg::create(MsgKind::ChannelClose, 0, 0, vec![]);
        if let Err(error) = self.write(&rq)
        {
            WARN!("Unable to write close message to tunnel server {}", error);
        }
        self.channel.as_ref()
            .ok_or("Channel is not created")?
            .shutdown(Shutdown::Both)?;
        Ok(())
    }

    pub fn on_message(& mut self, callback: &'a mut impl FnMut(&CallbackEvent) -> Option<Vec<Msg>> )
    {
        self.msg_handle = Some(callback);
    }

    fn get_poll(&mut self) -> Result<&mut Poll, Box<dyn Error>>
    {
        if let None = self.poll
        {
            self.poll = Some(Poll::new()?);
        }
        Ok(self.poll.as_mut().ok_or("Invalid poll object")?)
    }
    pub fn register_io(&mut self, fd: RawFd,  interest: IOInterest) -> Result<(), Box<dyn Error>>
    {
        // add socket to polling
        let token = Token(self.n_token);
        self.get_poll()?
            .registry()
            .register(&mut SourceFd(&fd), token, interest)?;
        // register the handle
        let _ = self.io_fds.insert(token, fd);
        self.n_token = self.n_token + 1;
        Ok(())
    }

    pub fn set_step_to(&mut self, to: Duration)
    {
        self.stepto = Some(to);
    }

    pub fn step(&mut self) -> Result<(), Box<dyn Error>>
    {
        // Poll Mio for events, blocking or timeout
        let mut events = Events::with_capacity(128);
        let timeout = self.stepto;
        self.get_poll()?
            .poll(&mut events, timeout)?;
        // Process each event.
        for event in events.iter() {
            // We can use the token we previously provided to `register` to
            // determine for which socket the event is.
            let mut evt = CallbackEvent::create(None, Some(event), None);
            let mut response = None;

            match event.token() {
                SERVER => {
                    let data = self.read()?;
                    evt.msg = Some(&data);
                    if let Some(callback) = self.msg_handle.as_mut()
                    {
                        response = callback(&evt);
                    }
                },
                token => {
                    evt.fd = self.io_fds.get(&token);
                    if let Some(callback) = self.msg_handle.as_mut()
                    {
                        response = callback(&evt);
                    }
                }
            }
            if let Some(msgs) = response
            {
                for msg in msgs.into_iter() {
                    self.write(&msg)?;
                }
            }
        }
        Ok(())
    }
}


impl<'a> Drop for Topic<'a>
{
    fn drop(&mut self)
    {
        INFO!("Closing topic: {}", self.name);
        if let Some(callback) = self.msg_handle.as_mut()
        {
            let rq = Msg::create(MsgKind::ChannelUnsubscribeAll, 0, 0, Vec::new());
            let evt = CallbackEvent::create(
                        None,
                        None,
                        Some(&rq));
            if let Some(msgs) = callback(&evt)
            {
                for msg in msgs.into_iter() {
                    INFO!("Write message to client ID {}", msg.client_id);
                   if let Err(error) = self.write(&msg)
                   {
                       ERROR!("unable to send message to topic [{}]: {}", self.name, error);
                   }
                }
            }
        }
        if let Err(error) = self.close()
        {
            ERROR!("Unable to close topic [{}]: {}", self.name, error);
        }
    }
}


impl Msg
{
    /// Create new `Msg` object
    ///
    /// Arguments
    ///
    /// * `kind` - message type
    /// * `channel_id` - the channel id
    /// * `client_id` - websocket client id
    /// * `data` - raw data buffer
    pub fn create(kind: MsgKind, channel_id: u16, client_id: u16, data: Vec<u8>) -> Self
    {
        Self { kind, channel_id, client_id, size:data.len() as u32, data: data }
    }

}



impl std::fmt::Display for Msg {
    ///  Implement Display trait for Msg
    ///
    /// Arguments
    ///
    /// * `f` -input formatter
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "Antunnel message dump:")?;
        writeln!(f, "Kind: {} - [{:#02x}]", self.kind, MsgKind::to_u8(&self.kind))?;
        writeln!(f, "Channel ID: {} - {}", self.channel_id, format!("{:#02x?}", self.channel_id.to_be_bytes()))?;
        writeln!(f, "Client ID: {} - {}", self.client_id, format!("{:#02x?}", self.client_id.to_be_bytes()))?;
        writeln!(f, "Data size: {} - {}", self.size, format!("{:#02x?}", self.size.to_be_bytes()))?;
        writeln!(f, "Data : {}", format!("{:#02x?}", self.data))
    }
}

impl std::fmt::Display for MsgKind {
    ///  Implement Display trait for Msgkind
    ///
    /// Arguments
    ///
    /// * `f` -input formatter
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = match self {
            MsgKind::ChannelOpen => 0x4,
            MsgKind::ChannelOk => 0x0,
            MsgKind::ChannelSubscribe => 0x2,
            MsgKind::ChannelUnsubscribe => 0x3,
            MsgKind::ChannelData => 0x6,
            MsgKind::ChannelError => 0x1,
            MsgKind::ChannelCtrl => 0x7,
            MsgKind::ChannelClose => 0x5,
            MsgKind::ChannelUnsubscribeAll => 0xA,
            MsgKind::Unknown => 0xFF,
        };
        write!(f, "{:#02x}", s)
    }
}