//! Implementation of the artnet protocol as a DmxPort.
use anyhow::{anyhow, Context};
use artnet_protocol::{ArtCommand, Output, Poll};
use log::{error, warn};
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
}

impl TryFrom<ArtnetDmxPortParams> for ArtnetDmxPort {
    type Error = anyhow::Error;
    fn try_from(params: ArtnetDmxPortParams) -> Result<Self, Self::Error> {
        Ok(Self {
            socket: get_socket()?,
            params,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct ArtnetDmxPortParams {
    addr: Ipv4Addr,
    short_name: String,
    long_name: String,
}

impl std::fmt::Display for ArtnetDmxPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ArtNet output {} at {} ({})",
            self.params.short_name, self.params.addr, self.params.long_name
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
    fn write(&mut self, frame: &[u8]) -> anyhow::Result<()> {
        let command = ArtCommand::Output(Output {
            data: frame.to_vec().into(),
            ..Default::default()
        });
        let buffer = command.write_to_buffer()?;
        let dest = SocketAddrV4::new(self.params.addr, PORT);
        self.socket.send_to(&buffer, dest)?;
        Ok(())
    }
}

#[typetag::serde]
impl DmxPort for ArtnetDmxPort {
    /// Poll for artnet devices
    fn available_ports(wait: Duration) -> anyhow::Result<PortListing> {
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
                ports.push(Box::new(Self {
                    socket: get_socket()?,
                    params: ArtnetDmxPortParams {
                        addr: reply.address,
                        short_name: null_terminated_string_lossy(&reply.short_name).to_string(),
                        long_name: null_terminated_string_lossy(&reply.long_name).to_string(),
                    },
                }) as Box<dyn DmxPort>);
            }
            Ok(())
        };

        loop {
            let waited_so_far = start.elapsed();
            if waited_so_far > wait {
                break;
            }
            if let Err(err) = receive_poll(wait - waited_so_far) {
                error!("Error receiving artnet poll response: {err}.");
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
