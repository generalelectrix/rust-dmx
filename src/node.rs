//! Implementation of the artnet protocol as a node: DMX arrives from the
//! network and goes out a DMX port.
use anyhow::{Context, Result};
use artnet_protocol::{ArtCommand, PollReply, PortAddress};
use log::{debug, error};

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use crate::artnet::{PORT, get_socket};
use crate::{DmxPort, available_ports};

/// The largest artnet packet a node will read.
///
/// A full-universe ArtDmx packet is 530 bytes; the rest is headroom for the
/// larger management packets a node reads far enough to identify and discard.
const MAX_PACKET_SIZE: usize = 1024;

/// How long a serving node waits for a packet before it reconsiders whether to
/// keep running.
const SERVE_TIMEOUT: Duration = Duration::from_millis(250);

/// The port type of an output that can emit DMX from artnet.
const PORT_TYPE_OUTPUT: u8 = 0x80;

/// The good-output flag indicating that data is being transmitted.
const GOOD_OUTPUT_DATA_TRANSMITTED: u8 = 0x80;

/// The style code for a node that converts artnet to DMX.
const STYLE_NODE: u8 = 0x00;

/// The bind index of a device that is not part of a larger product.
///
/// Bind indices order the devices of a modular product from its root outward,
/// and a standalone node is its own root.
const BIND_INDEX_ROOT: u8 = 1;

/// How a node presents itself to controllers browsing the network.
#[derive(Debug, Clone)]
pub struct ArtnetNodeConfig {
    /// The artnet port address whose DMX this node outputs.
    pub port_address: PortAddress,
    /// The name a controller lists this node under. Truncated to 17 characters.
    pub short_name: String,
    /// The description a controller shows for this node. Truncated to 63 characters.
    pub long_name: String,
}

/// An artnet node that forwards one universe to a DMX port.
///
/// The node answers polls so controllers can discover it, and writes the DMX
/// payload of every frame addressed to its universe straight through to the
/// port. It does not buffer or re-clock frames: a DMX interface runs its own
/// output clock and repeats the last frame it was given, so writing on arrival
/// produces correct output at the lowest latency.
///
/// A node reads every artnet packet arriving at the socket it serves. Only one
/// reader of a given socket can see a particular packet, so a running node and
/// a concurrent [`crate::available_ports`] browse over the same socket will
/// take packets from each other.
pub struct ArtnetNode {
    socket: UdpSocket,
    config: ArtnetNodeConfig,
    port: Box<dyn DmxPort>,
    recv_buf: Box<[u8; MAX_PACKET_SIZE]>,
}

/// What a node did with a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// A frame for this node's universe went out the DMX port.
    Output,
    /// A poll was answered.
    Polled,
    /// Nothing this node acts on.
    Ignored,
}

impl ArtnetNode {
    /// Serve the provided DMX port on the standard artnet UDP port.
    ///
    /// Every node and artnet port in a process shares one socket on that port.
    pub fn new(port: Box<dyn DmxPort>, config: ArtnetNodeConfig) -> Result<Self> {
        let socket = get_socket()?;
        Ok(Self::with_socket(socket, port, config))
    }

    /// Serve the provided DMX port on an already-bound socket.
    ///
    /// The node reads and writes the socket as given; it does not rebind it or
    /// alter its address.
    pub fn with_socket(
        socket: UdpSocket,
        port: Box<dyn DmxPort>,
        config: ArtnetNodeConfig,
    ) -> Self {
        Self {
            socket,
            config,
            port,
            recv_buf: Box::new([0; MAX_PACKET_SIZE]),
        }
    }

    /// Announce this node to every controller on the network.
    ///
    /// A controller learns of a node by polling for one, so a node that appears
    /// after a controller has polled goes unseen until the next poll. An
    /// announcement makes it visible right away.
    pub fn announce(&mut self) -> Result<()> {
        self.socket
            .set_broadcast(true)
            .context("setting artnet socket to allow broadcast")?;
        self.send_poll_reply(SocketAddr::from((Ipv4Addr::BROADCAST, PORT)))
    }

    /// Receive one packet and act on it.
    ///
    /// Blocks until a packet arrives or the socket's read timeout runs out.
    /// A packet that is not artnet, or that the DMX port refuses, is an error.
    pub fn serve_one(&mut self) -> Result<()> {
        let (length, source) = self.socket.recv_from(self.recv_buf.as_mut_slice())?;
        let command = ArtCommand::from_buffer(&self.recv_buf[..length])?;
        self.dispatch(command, source)?;
        Ok(())
    }

    /// Serve packets until `should_run` returns false.
    ///
    /// A packet this node cannot parse or cannot output is logged and skipped;
    /// only `should_run` ends the loop. It is consulted between packets and
    /// again each time the wait for one runs out, so a node receiving no
    /// traffic still stops.
    pub fn run(&mut self, should_run: impl Fn() -> bool) {
        if let Err(err) = self.socket.set_read_timeout(Some(SERVE_TIMEOUT)) {
            error!(
                "Could not set a timeout on the artnet node socket: {err:#}. \
                The node will keep serving until a packet arrives after it is asked to stop."
            );
        }
        while should_run() {
            if let Err(err) = self.serve_one() {
                if is_timeout(&err) {
                    continue;
                }
                debug!("Artnet node dropped a packet: {err:#}.");
            }
        }
    }

    /// Act on one parsed command received from `source`.
    fn dispatch(&mut self, command: ArtCommand, source: SocketAddr) -> Result<Disposition> {
        match command {
            ArtCommand::Output(output) if output.port_address == self.config.port_address => {
                self.port.write(output.data.as_ref())?;
                Ok(Disposition::Output)
            }
            ArtCommand::Poll(_) => {
                self.send_poll_reply(source)?;
                Ok(Disposition::Polled)
            }
            _ => Ok(Disposition::Ignored),
        }
    }

    /// Send this node's poll reply to `dest`.
    fn send_poll_reply(&self, dest: SocketAddr) -> Result<()> {
        let reply = self.poll_reply(local_address_toward(dest.ip()));
        let buf = ArtCommand::PollReply(Box::new(reply))
            .write_to_buffer()
            .context("writing artnet poll reply")?;
        self.socket
            .send_to(&buf, dest)
            .context("sending artnet poll reply")?;
        Ok(())
    }

    /// This node's poll reply, reporting itself at `address`.
    fn poll_reply(&self, address: Ipv4Addr) -> PollReply {
        let fields = split_port_address(self.config.port_address);
        PollReply {
            address,
            port: PORT,
            port_address: fields.node,
            // A single output, on the universe this node serves.
            num_ports: [0, 1],
            port_types: [PORT_TYPE_OUTPUT, 0, 0, 0],
            good_output: [GOOD_OUTPUT_DATA_TRANSMITTED, 0, 0, 0],
            swout: [fields.universe, 0, 0, 0],
            short_name: null_terminated(&self.config.short_name),
            long_name: null_terminated(&self.config.long_name),
            style: STYLE_NODE,
            bind_index: BIND_INDEX_ROOT,
            // The artnet specification permits a node that cannot supply its
            // MAC address to report zero.
            mac: [0; 6],
            ..Default::default()
        }
    }
}

/// The first DMX port attached to this system, if there is one.
///
/// Artnet ports are excluded. A node forwarding to one would put the frames it
/// just received back on the network, addressed to another node.
pub fn first_available_port() -> Result<Option<Box<dyn DmxPort>>> {
    Ok(available_ports(None)?.into_iter().next())
}

/// The halves a 15-bit port address splits into in a poll reply.
///
/// Net (bits 14-8) and Sub-Net (bits 7-4) belong to the node as a whole; the
/// Universe (bits 3-0) belongs to an individual output port.
struct PortAddressFields {
    /// The node's Net and Sub-Net switches.
    node: [u8; 2],
    /// The output port's Universe switch.
    universe: u8,
}

/// Split a port address into the fields a poll reply carries it in.
fn split_port_address(addr: PortAddress) -> PortAddressFields {
    let addr = u16::from(addr);
    PortAddressFields {
        node: [((addr >> 8) & 0x7F) as u8, ((addr >> 4) & 0x0F) as u8],
        universe: (addr & 0x0F) as u8,
    }
}

/// The local address that reaches `peer`.
///
/// Falls back to the unspecified address when no route can be found, which a
/// controller reads as "ask the network layer where this came from".
fn local_address_toward(peer: IpAddr) -> Ipv4Addr {
    fn probe(peer: IpAddr) -> Result<Ipv4Addr> {
        // Connecting a UDP socket sends nothing; it just asks the routing table
        // which local address would carry a packet to this peer.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        socket.connect((peer, PORT))?;
        match socket.local_addr()?.ip() {
            IpAddr::V4(addr) => Ok(addr),
            IpAddr::V6(addr) => Err(anyhow::anyhow!("bound an IPv6 address: {addr}")),
        }
    }
    match probe(peer) {
        Ok(addr) => addr,
        Err(err) => {
            debug!("Could not determine the local address toward {peer}: {err:#}.");
            Ipv4Addr::UNSPECIFIED
        }
    }
}

/// Fit a name into a fixed-size null-terminated artnet string field.
///
/// A name too long for the field is truncated at a character boundary, leaving
/// room for the null.
fn null_terminated<const N: usize>(name: &str) -> [u8; N] {
    let mut field = [0u8; N];
    let mut len = name.len().min(N - 1);
    while len > 0 && !name.is_char_boundary(len) {
        len -= 1;
    }
    field[..len].copy_from_slice(&name.as_bytes()[..len]);
    field
}

/// Whether this error is a socket read that ran out its timeout.
fn is_timeout(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<std::io::Error>().map(|e| e.kind()),
        Some(ErrorKind::WouldBlock | ErrorKind::TimedOut)
    )
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ArtnetDmxPort;
    use crate::artnet::{null_terminated_string_lossy, output_port_address};
    use crate::{OpenError, WriteError};
    use artnet_protocol::Output;
    use serde::{Deserialize, Serialize};
    use std::sync::{Arc, Mutex};

    /// A DMX port that records the frames written to it.
    #[derive(Debug, Default, Serialize, Deserialize)]
    struct RecordingDmxPort {
        #[serde(skip)]
        frames: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    #[typetag::serde]
    impl DmxPort for RecordingDmxPort {
        fn open(&mut self) -> Result<(), OpenError> {
            Ok(())
        }

        fn close(&mut self) {}

        fn write(&mut self, frame: &[u8]) -> Result<(), WriteError> {
            self.frames.lock().unwrap().push(frame.to_vec());
            Ok(())
        }
    }

    impl std::fmt::Display for RecordingDmxPort {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "recording")
        }
    }

    /// A node serving `universe` on a loopback socket, and the frames it outputs.
    fn node(universe: u16) -> (ArtnetNode, Arc<Mutex<Vec<Vec<u8>>>>) {
        let port = RecordingDmxPort::default();
        let frames = port.frames.clone();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let node = ArtnetNode::with_socket(
            socket,
            Box::new(port),
            ArtnetNodeConfig {
                port_address: universe.try_into().unwrap(),
                short_name: "test node".into(),
                long_name: "a node built for a test".into(),
            },
        );
        (node, frames)
    }

    /// An ArtDmx packet addressed to `universe`, as it appears on the wire.
    fn dmx_packet(universe: u16, data: Vec<u8>) -> Vec<u8> {
        ArtCommand::Output(Output {
            port_address: universe.try_into().unwrap(),
            data: data.into(),
            ..Default::default()
        })
        .write_to_buffer()
        .unwrap()
    }

    /// Parse a wire packet the way a node does.
    fn parse(packet: &[u8]) -> Result<ArtCommand> {
        Ok(ArtCommand::from_buffer(packet)?)
    }

    /// An address for packets whose source does not matter to the test.
    fn nowhere() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, PORT))
    }

    /// The universe a poll reply advertises on its first output port, decoded by
    /// the same code a controller browsing this node would use.
    fn advertised_universe(reply: &PollReply) -> u16 {
        output_port_address(reply, 0)
    }

    #[test]
    fn advertises_the_universe_it_serves() {
        // Every part of the 15-bit address has to survive the split into the
        // node's Net and Sub-Net switches and the output's Universe switch.
        for universe in [0, 1, 15, 1 << 4, (1 << 8) | (2 << 4) | 3, 0x7FFF] {
            let (node, _) = node(universe);
            let reply = node.poll_reply(Ipv4Addr::LOCALHOST);
            assert_eq!(advertised_universe(&reply), universe);
        }
    }

    #[test]
    fn advertises_a_single_dmx_output() {
        let (node, _) = node(1);
        let reply = node.poll_reply(Ipv4Addr::LOCALHOST);
        assert_eq!(reply.num_ports, [0, 1]);
        // Exactly one port, and it can output DMX from artnet.
        assert_ne!(reply.port_types[0] & PORT_TYPE_OUTPUT, 0);
        assert_eq!(reply.port_types[1..], [0, 0, 0]);
        assert_eq!(reply.port, PORT);
        assert_eq!(reply.style, STYLE_NODE);
        // A standalone node is the root of its own product.
        assert_eq!(reply.bind_index, BIND_INDEX_ROOT);
    }

    #[test]
    fn advertises_its_configured_names() {
        let (node, _) = node(1);
        let reply = node.poll_reply(Ipv4Addr::LOCALHOST);
        assert_eq!(null_terminated_string_lossy(&reply.short_name), "test node");
        assert_eq!(
            null_terminated_string_lossy(&reply.long_name),
            "a node built for a test"
        );
    }

    #[test]
    fn truncates_names_too_long_for_their_field() {
        let long = "x".repeat(100);
        let short_name: [u8; 18] = null_terminated(&long);
        let long_name: [u8; 64] = null_terminated(&long);
        // The field is filled to its last byte, which stays null.
        assert_eq!(short_name[16], b'x');
        assert_eq!(short_name[17], 0);
        assert_eq!(long_name[62], b'x');
        assert_eq!(long_name[63], 0);

        // A name is never cut in the middle of a character.
        let multibyte: [u8; 18] = null_terminated(&"é".repeat(20));
        assert_eq!(null_terminated_string_lossy(&multibyte), "é".repeat(8));
    }

    #[test]
    fn a_controller_browsing_this_node_finds_its_output() {
        // Net 1, sub-net 2, universe 3: every field of the address in play.
        let universe = (1 << 8) | (2 << 4) | 3;
        let (node, _) = node(universe);
        let reply = node.poll_reply(Ipv4Addr::new(10, 0, 0, 7));

        // Enumerated by the same code a controller uses to browse for nodes.
        let ports = ArtnetDmxPort::ports_from_poll(&reply).unwrap();
        assert_eq!(ports.len(), 1);
        assert_eq!(
            ports[0].to_string(),
            format!("ArtNet test node at 10.0.0.7 universe {universe} (a node built for a test)")
        );
    }

    #[test]
    fn outputs_frames_for_its_own_universe() {
        let (mut node, frames) = node(3);
        let payload: Vec<u8> = (0..512).map(|i| i as u8).collect();

        let packet = dmx_packet(3, payload.clone());
        assert_eq!(
            node.dispatch(parse(&packet).unwrap(), nowhere()).unwrap(),
            Disposition::Output
        );
        assert_eq!(*frames.lock().unwrap(), vec![payload]);
    }

    #[test]
    fn ignores_frames_for_other_universes() {
        let (mut node, frames) = node(3);
        for universe in [0, 2, 4, 0x7FFF] {
            let packet = dmx_packet(universe, vec![1, 2, 3, 4]);
            assert_eq!(
                node.dispatch(parse(&packet).unwrap(), nowhere()).unwrap(),
                Disposition::Ignored
            );
        }
        assert!(frames.lock().unwrap().is_empty());
    }

    #[test]
    fn ignores_commands_it_does_not_implement() {
        let (mut node, frames) = node(3);
        let reply = ArtCommand::PollReply(Box::default());
        assert_eq!(
            node.dispatch(reply, nowhere()).unwrap(),
            Disposition::Ignored
        );
        assert!(frames.lock().unwrap().is_empty());
    }

    #[test]
    fn rejects_packets_that_are_not_artnet() {
        use artnet_protocol::Error;

        let short = parse(b"nope").unwrap_err();
        assert!(
            matches!(
                short.downcast_ref::<Error>(),
                Some(Error::MessageTooShort { min_len: 14, .. })
            ),
            "expected a too-short message, got: {short:#}"
        );

        let not_artnet = parse(&[b'N'; 32]).unwrap_err();
        assert!(
            matches!(
                not_artnet.downcast_ref::<Error>(),
                Some(Error::InvalidArtnetHeader(_))
            ),
            "expected an invalid header, got: {not_artnet:#}"
        );

        // A well-formed header carrying an opcode no node acts on.
        let mut unknown = dmx_packet(1, vec![0; 24]);
        unknown[8..10].copy_from_slice(&0x1234u16.to_le_bytes());
        let unknown = parse(&unknown).unwrap_err();
        assert!(
            matches!(
                unknown.downcast_ref::<Error>(),
                Some(Error::UnknownOpcode(0x1234))
            ),
            "expected an unknown opcode, got: {unknown:#}"
        );
    }

    #[test]
    fn serves_the_next_frame_after_a_bad_packet() {
        let (mut node, frames) = node(1);
        let addr = node.socket.local_addr().unwrap();
        let controller = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        node.socket.set_read_timeout(Some(SERVE_TIMEOUT)).unwrap();

        controller
            .send_to(b"this is not artnet at all", addr)
            .unwrap();
        assert!(node.serve_one().is_err());

        controller
            .send_to(&dmx_packet(1, vec![7; 24]), addr)
            .unwrap();
        node.serve_one().unwrap();
        assert_eq!(*frames.lock().unwrap(), vec![vec![7; 24]]);
    }

    #[test]
    fn answers_a_poll_from_the_controller_that_sent_it() {
        let (mut node, _) = node(9);
        let node_addr = node.socket.local_addr().unwrap();
        let controller = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        controller.set_read_timeout(Some(SERVE_TIMEOUT)).unwrap();
        node.socket.set_read_timeout(Some(SERVE_TIMEOUT)).unwrap();

        let poll = ArtCommand::Poll(Default::default())
            .write_to_buffer()
            .unwrap();
        controller.send_to(&poll, node_addr).unwrap();
        node.serve_one().unwrap();

        let mut buf = [0u8; MAX_PACKET_SIZE];
        let (length, from) = controller.recv_from(&mut buf).unwrap();
        assert_eq!(from, node_addr);
        let ArtCommand::PollReply(reply) = ArtCommand::from_buffer(&buf[..length]).unwrap() else {
            panic!("a poll was answered with something other than a poll reply");
        };
        assert_eq!(advertised_universe(&reply), 9);
        assert_ne!(reply.port_types[0] & PORT_TYPE_OUTPUT, 0);
    }

    #[test]
    fn stops_serving_when_told_to() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let (mut node, _) = node(1);
        let run = Arc::new(AtomicBool::new(true));
        let flag = run.clone();
        let serving = std::thread::spawn(move || node.run(|| flag.load(Ordering::Relaxed)));

        // Wait until the node is parked waiting for a packet that will never
        // arrive. From there only its read timeout can return it to the top of
        // the loop, where it can see that it has been stopped.
        std::thread::sleep(SERVE_TIMEOUT * 2);
        run.store(false, Ordering::Relaxed);

        let deadline = std::time::Instant::now() + SERVE_TIMEOUT * 8;
        while !serving.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            serving.is_finished(),
            "the node kept serving after it was stopped"
        );
        serving.join().unwrap();
    }
}
