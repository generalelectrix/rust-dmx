//! Implementation of the artnet protocol as a DmxPort.
use anyhow::{Context, Result, anyhow};
use artnet_protocol::{ArtCommand, Poll, PollReply};
use log::{debug, warn};
use serde::{Deserialize, Serialize};

use std::{
    net::{Ipv4Addr, SocketAddrV4, ToSocketAddrs, UdpSocket},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{DmxPort, PortListing};

pub(crate) const PORT: u16 = 6454;

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
            "ArtNet {} at {} universe {} ({})",
            self.params.short_name,
            self.params.addr,
            self.params.port_address,
            self.params.long_name
        )
    }
}

// TODO: replace with OnceLock once the fallible init API is stabilized.
static ARTNET_SOCKET: Mutex<Option<UdpSocket>> = Mutex::new(None);

pub(crate) fn get_socket() -> anyhow::Result<UdpSocket> {
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
    /// Poll for artnet devices. Continue polling for the provided timeout.
    pub fn available_ports(wait: Duration) -> Result<PortListing> {
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

        // Collected across every reply, including the separate replies that a
        // multi-port node pages its ports across (Art-Net "BindIndex" paging),
        // so a gateway with more than four outputs is fully enumerated.
        let mut ports: Vec<Self> = vec![];

        let mut receive_poll = |timeout| -> anyhow::Result<()> {
            socket.set_read_timeout(Some(timeout))?;
            let mut buffer = [0u8; 1024];
            let (length, _addr) = socket.recv_from(&mut buffer)?;
            let command = ArtCommand::from_buffer(&buffer[..length])?;

            if let ArtCommand::PollReply(reply) = command {
                ports.extend(Self::ports_from_poll(&reply)?);
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

        let listing = Self::sorted_unique(ports)
            .into_iter()
            .map(|p| Box::new(p) as Box<dyn DmxPort>)
            .collect();
        Ok(listing)
    }

    /// Order ports by destination - node address then universe - and keep one
    /// port per distinct destination.
    ///
    /// Replies arrive in nondeterministic order and a node answers a poll more
    /// than once, so an unprocessed listing is neither stable nor unique.
    fn sorted_unique(mut ports: Vec<Self>) -> Vec<Self> {
        ports.sort_by_key(|p| (p.params.addr, p.params.port_address));
        ports.dedup_by_key(|p| (p.params.addr, p.params.port_address));
        ports
    }

    /// One port for each DMX output a node advertises in a poll reply.
    ///
    /// A node marks which of its (up to four) ports are outputs with the output
    /// bit of each `port_types` entry, and gives the universe each output
    /// listens on in the matching `swout` entry.
    pub(crate) fn ports_from_poll(reply: &PollReply) -> Result<Vec<Self>> {
        let mut ports = Vec::new();
        for i in 0..4 {
            // Bit 7 of a port type set means the port can output DMX from Art-Net.
            if reply.port_types[i] & 0x80 == 0 {
                continue;
            }
            ports.push(Self {
                socket: get_socket()?,
                params: ArtnetDmxPortParams {
                    addr: reply.address,
                    port_address: output_port_address(reply, i),
                    short_name: null_terminated_string_lossy(&reply.short_name).to_string(),
                    long_name: null_terminated_string_lossy(&reply.long_name).to_string(),
                },
                send_buf: vec![],
            });
        }
        Ok(ports)
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
    fn open(&mut self) -> Result<(), crate::OpenError> {
        Ok(())
    }

    fn close(&mut self) {}

    fn write(&mut self, frame: &[u8]) -> Result<(), crate::WriteError> {
        self.write(frame)?;
        Ok(())
    }
}

/// The 15-bit Art-Net Port-Address of one of a node's output ports.
///
/// Net (bits 14-8) and Sub-Net (bits 7-4) are shared by the whole node and
/// carried in `port_address`; the Universe (bits 3-0) is per output port and
/// carried in the matching `swout` entry.
pub(crate) fn output_port_address(reply: &PollReply, output_index: usize) -> u16 {
    let net = (reply.port_address[0] & 0x7F) as u16;
    let sub_net = (reply.port_address[1] & 0x0F) as u16;
    let universe = (reply.swout[output_index] & 0x0F) as u16;
    (net << 8) | (sub_net << 4) | universe
}

pub(crate) fn null_terminated_string_lossy(bytes: &[u8]) -> String {
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
    use anyhow::{Result, ensure};

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
        let add_pad_byte = !buf.len().is_multiple_of(2);
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

#[cfg(test)]
mod test {
    use super::*;
    use artnet_protocol::PollReply;

    /// The 15-bit universe each enumerated output port targets, in order.
    fn universes(reply: &PollReply) -> Vec<u16> {
        ArtnetDmxPort::ports_from_poll(reply)
            .unwrap()
            .iter()
            .map(|p| p.params.port_address)
            .collect()
    }

    #[test]
    fn enumerates_advertised_output_universes() {
        // Two outputs on net 0 / sub-net 0 listening on universes 1 and 2.
        let reply = PollReply {
            port_types: [0x80, 0x80, 0, 0],
            swout: [1, 2, 0, 0],
            ..Default::default()
        };
        assert_eq!(universes(&reply), vec![1, 2]);

        // The net and sub-net switches occupy the high bits of the universe:
        // net in bits 14-8, sub-net in bits 7-4, the swout nibble in bits 3-0.
        let reply = PollReply {
            port_address: [0x01, 0x02], // NetSwitch 1, SubSwitch 2
            port_types: [0x80, 0, 0, 0],
            swout: [3, 0, 0, 0],
            ..Default::default()
        };
        assert_eq!(universes(&reply), vec![(1 << 8) | (2 << 4) | 3]);
    }

    #[test]
    fn skips_non_output_ports() {
        // Port 0 is input-only and skipped; ports 1 (in+out) and 2 (out) are kept.
        let reply = PollReply {
            port_types: [0x40, 0xC0, 0x80, 0x00],
            swout: [9, 5, 6, 7],
            ..Default::default()
        };
        assert_eq!(universes(&reply), vec![5, 6]);
    }

    #[test]
    fn sorts_and_dedupes_destinations_across_replies() {
        let page = |addr: [u8; 4], sub_net: u8, swout: [u8; 4]| PollReply {
            address: addr.into(),
            port_address: [0, sub_net],
            port_types: [0x80, 0x80, 0, 0],
            swout,
            ..Default::default()
        };
        let lo = [10, 0, 0, 5];
        let hi = [10, 0, 0, 7];
        // Replies land out of order: a higher node first, a node paging its
        // outputs across two sub-nets, and a re-sent page. The result is sorted
        // by node then universe, with each destination appearing once.
        let ports = [
            page(hi, 0, [4, 3, 0, 0]),
            page(lo, 1, [2, 1, 0, 0]),
            page(lo, 0, [2, 1, 0, 0]),
            page(lo, 1, [2, 1, 0, 0]),
        ]
        .iter()
        .flat_map(|r| ArtnetDmxPort::ports_from_poll(r).unwrap())
        .collect();
        let destinations: Vec<(Ipv4Addr, u16)> = ArtnetDmxPort::sorted_unique(ports)
            .iter()
            .map(|p| (p.params.addr, p.params.port_address))
            .collect();
        assert_eq!(
            destinations,
            vec![
                (lo.into(), 1),
                (lo.into(), 2),
                (lo.into(), (1 << 4) | 1),
                (lo.into(), (1 << 4) | 2),
                (hi.into(), 3),
                (hi.into(), 4),
            ]
        );
    }
}
