//! Implementation of the artnet protocol as a DmxPort.
use anyhow::{anyhow, Context, Result};
use artnet_protocol::{ArtCommand, Poll, PollReply};
use log::{debug, warn};
use serde::{Deserialize, Serialize};

use std::{
    net::{Ipv4Addr, SocketAddrV4, ToSocketAddrs, UdpSocket},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{DmxPort, PortListing};

const PORT: u16 = 6454;

#[derive(Serialize, Deserialize)]
#[serde(try_from = "ArtnetDmxPortParams")]
pub struct ArtnetDmxPort {
    #[serde(skip_serializing)]
    socket: UdpSocket,
    #[serde(flatten)]
    params: ArtnetDmxPortParams,
    #[serde(skip_serializing)]
    send_buf: Vec<u8>,
}

impl TryFrom<ArtnetDmxPortParams> for ArtnetDmxPort {
    type Error = anyhow::Error;
    fn try_from(params: ArtnetDmxPortParams) -> Result<Self, Self::Error> {
        Ok(Self {
            socket: get_socket()?,
            params,
            send_buf: vec![],
        })
    }
}

#[derive(Serialize, Deserialize)]
struct ArtnetDmxPortParams {
    addr: Ipv4Addr,
    /// The artnet internal port address.
    port_address: u16,
    short_name: String,
    long_name: String,
}

impl std::fmt::Display for ArtnetDmxPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ArtNet output {} at {} (port {}) ({})",
            self.params.short_name,
            self.params.addr,
            self.params.port_address,
            self.params.long_name
        )
    }
}

// TODO: replace with OnceLock once the fallible init API is stabilized.
static ARTNET_SOCKET: Mutex<Option<UdpSocket>> = Mutex::new(None);

fn get_socket() -> anyhow::Result<UdpSocket> {
    let mut socket_guard = ARTNET_SOCKET
        .lock()
        .map_err(|_| anyhow!("failed to acquire global artnet socket lock"))?;
    if let Some(s) = socket_guard.as_ref() {
        return s.try_clone().context("cloning artnet socket");
    }

    let s = UdpSocket::bind(("0.0.0.0", PORT)).context("failed to bind UDP socket for artnet")?;
    let cloned = s.try_clone().context("cloning artnet socket")?;
    *socket_guard = Some(s);
    Ok(cloned)
}

impl ArtnetDmxPort {
    fn from_poll(reply: &PollReply) -> Result<Self> {
        Ok(Self {
            socket: get_socket()?,
            params: ArtnetDmxPortParams {
                addr: reply.address,
                port_address: u16::from_be_bytes(reply.port_address),
                short_name: null_terminated_string_lossy(&reply.short_name).to_string(),
                long_name: null_terminated_string_lossy(&reply.long_name).to_string(),
            },
            send_buf: vec![],
        })
    }

    fn write(&mut self, frame: &[u8]) -> Result<()> {
        // TODO: the first section of the packet is always the same
        // we could pre-populate that. Probably not important, its a handful of
        // bytes at most.
        self.send_buf.clear();
        send::write(&mut self.send_buf, self.params.port_address, frame)
            .context("constructing artnet buffer")?;
        let dest = SocketAddrV4::new(self.params.addr, PORT);
        self.socket.send_to(&self.send_buf, dest)?;
        Ok(())
    }
}

#[typetag::serde]
impl DmxPort for ArtnetDmxPort {
    /// Poll for artnet devices
    fn available_ports(wait: Duration) -> Result<PortListing> {
        let socket = get_socket()?;

        let broadcast_addr = ("255.255.255.255", PORT)
            .to_socket_addrs()
            .unwrap()
            .next()
            .unwrap();
        socket
            .set_broadcast(true)
            .context("setting ArtNet socket to allow broadcast")?;
        let buff = ArtCommand::Poll(Poll::default())
            .write_to_buffer()
            .context("writing ArtNet poll command")?;
        socket
            .send_to(&buff, broadcast_addr)
            .context("sending ArtNet poll message")?;

        let start = Instant::now();

        let mut ports = vec![];

        let mut receive_poll = |timeout| -> anyhow::Result<()> {
            socket.set_read_timeout(Some(timeout))?;
            let mut buffer = [0u8; 1024];
            let (length, _addr) = socket.recv_from(&mut buffer)?;
            let command = ArtCommand::from_buffer(&buffer[..length])?;

            if let ArtCommand::PollReply(reply) = command {
                ports.push(Box::new(Self::from_poll(&reply)?) as Box<dyn DmxPort>);
            }
            Ok(())
        };

        loop {
            let waited_so_far = start.elapsed();
            if waited_so_far > wait {
                break;
            }
            if let Err(err) = receive_poll(wait - waited_so_far) {
                debug!("Error receiving artnet poll response: {err}.");
            }
        }
        if let Err(err) = socket.set_read_timeout(None) {
            warn!("Error disabling ArtNet socket timeout: {err}");
        }
        Ok(ports)
    }

    fn open(&mut self) -> Result<(), crate::OpenError> {
        Ok(())
    }

    fn close(&mut self) {}

    fn write(&mut self, frame: &[u8]) -> Result<(), crate::WriteError> {
        self.write(frame)?;
        Ok(())
    }
}

fn null_terminated_string_lossy(bytes: &[u8]) -> String {
    let null_pos = bytes
        .iter()
        .position(|c| *c == b'\0')
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[0..null_pos]).to_string()
}

mod send {
    //! The artnet_protocol library is way too eager to allocate memory on every
    //! write to the port. This is a common issue with libraries that try to
    //! represent an API as an enum (see also: the OSC library).

    //! This little module implements just the code we need to write an artnet
    //! DMX packet, with no allocations.
    use anyhow::{ensure, Result};

    use std::io::Write;

    const ARTNET_HEADER: &[u8; 8] = b"Art-Net\0";
    const ARTNET_PROTOCOL_VERSION: [u8; 2] = [0, 14];

    /// Write the provided DMX buffer into the provided writer.
    ///
    /// The packet is addressed to the specified port address.
    pub fn write(mut w: impl Write, arnet_port_address: u16, buf: &[u8]) -> Result<()> {
        ensure!(!buf.is_empty(), "cannot send zero-length artnet frame");
        ensure!(
            buf.len() <= 512,
            "artnet frame payload too long: {}",
            buf.len()
        );

        let opcode: u16 = 0x5000;

        w.write_all(ARTNET_HEADER)?;
        // DMX output opcode.
        w.write_all(&opcode.to_le_bytes())?;
        w.write_all(&ARTNET_PROTOCOL_VERSION)?;
        // Packet sequence number - we only care about intranet so always write 0.
        write_u8(&mut w, 0)?;
        // Physical input port number - not used, write 0.
        write_u8(&mut w, 0)?;
        // Destination port number.
        w.write_all(&arnet_port_address.to_le_bytes())?;
        let add_pad_byte = buf.len() % 2 != 0;
        // Data payload length, rounded up to be a multiple of 2.
        let padded_len = buf.len() as u16 + add_pad_byte as u16;
        w.write_all(&padded_len.to_be_bytes())?;
        w.write_all(buf)?;
        if add_pad_byte {
            write_u8(&mut w, 0)?;
        }
        Ok(())
    }

    fn write_u8(mut w: impl Write, v: u8) -> std::io::Result<()> {
        let buf: [u8; 1] = [v];
        w.write_all(&buf)
    }

    #[cfg(test)]
    mod test {
        use artnet_protocol::{ArtCommand, Output};

        use super::write;
        /// Ensure that our hacked-together write method produces identical results
        /// as the artnet protocol library.
        #[test]
        fn test_match() {
            for len in 1..512 {
                let buf = vec![0u8; len];
                assert_match(&buf);
            }
        }

        fn write_vec(buf: &[u8]) -> Vec<u8> {
            let mut w = vec![];
            write(&mut w, 1, buf).unwrap();
            w
        }

        fn assert_match(buf: &[u8]) {
            let custom = write_vec(buf);
            let library = ArtCommand::Output(Output {
                data: buf.to_vec().into(),
                ..Default::default()
            })
            .write_to_buffer()
            .unwrap();
            assert_eq!(library, custom);
        }
    }
}
