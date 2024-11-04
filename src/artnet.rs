//! Implementation of the artnet protocol as a DmxPort.
use anyhow::Context;
use artnet_protocol::{ArtCommand, Poll}
use serde::{Deserialize, Serialize};

use std::{
    net::{Ipv4Addr, ToSocketAddrs, UdpSocket}, sync::OnceLock, time::{Duration, Instant}
};

use crate::{DmxPort, PortListing};

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

static ARTNET_SOCKET: OnceLock<UdpSocket> = OnceLock::new();

fn get_socket() -> anyhow::Result<UdpSocket> {
    Ok(ARTNET_SOCKET.get_or_init(|| UdpSocket::bind(("0.0.0.0", 6454)).expect("failed to bind UDP socket for ArtNet")).try_clone().context("cloning ArtNet socket")?)
}

impl DmxPort for ArtnetDmxPort {
    /// Poll for artnet devices
    fn available_ports(wait: Duration) -> anyhow::Result<PortListing> {
        let socket = get_socket()?;

        let broadcast_addr = ("255.255.255.255", 6454)
            .to_socket_addrs()
            .unwrap()
            .next()
            .unwrap();
        socket.set_broadcast(true).context("setting ArtNet socket to allow broadcast")?;
        let buff = ArtCommand::Poll(Poll::default()).write_to_buffer().context("writing ArtNet poll command")?;
        socket.send_to(&buff, &broadcast_addr).context("sending ArtNet poll message")?;

        let start = Instant::now();

        let mut ports = vec![];

        loop {
            let waited_to_far = start.elapsed();
            if waited_to_far > wait {
                break;
            }
            let timeout = wait - waited_to_far;
            socket.set_read_timeout(Some(timeout));
            let mut buffer = [0u8; 1024];
            let (length, _addr) = socket.recv_from(&mut buffer).unwrap();
            let command = ArtCommand::from_buffer(&buffer[..length]).unwrap();

            match command {
                ArtCommand::PollReply(reply) => {
                    ports.push(Box::new(Self {
                        socket: get_socket()?,
                        params: ArtnetDmxPortParams {                         addr: reply.address,
                            short_name: null_terminated_string_lossy(&reply.short_name).to_string(),
                            long_name: null_terminated_string_lossy(&reply.long_name).to_string(), }

                    }) as Box<dyn DmxPort>);
                }
                _ => {}
            }
        }
        socket.set_read_timeout(None);
        Ok(ports)
    }
}

fn null_terminated_string_lossy(bytes: &[u8]) -> String {
    let null_pos = bytes
        .iter()
        .position(|c| *c == b'\0')
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[0..null_pos]).to_string()
}
