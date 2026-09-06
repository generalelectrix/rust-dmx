//! Measure what it costs to run an artnet node.
//!
//! A node's whole job is to move one universe from the network to a DMX port,
//! so the question its cost answers is whether the machine it borrows time from
//! will notice. The stages below are the work of a single frame, timed
//! separately and then together, and reported against the rate an artnet
//! controller actually sends.
use std::hint::black_box;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use artnet_protocol::{ArtCommand, Output};
use rust_dmx::{ArtnetNode, ArtnetNodeConfig, DmxPort, OfflineDmxPort};

/// The rate a controller streams a universe at.
///
/// The artnet specification caps a stream at 44 frames per second.
const ARTNET_RATE: f64 = 44.0;

/// The universe used throughout.
const UNIVERSE: u16 = 1;

/// How long each stage is sampled for.
const SAMPLE_WINDOW: Duration = Duration::from_millis(300);

/// The most samples any one stage collects.
const MAX_SAMPLES: usize = 20_000;

/// Iterations run before sampling begins.
const WARMUP: usize = 20;

fn main() {
    if cfg!(debug_assertions) {
        eprintln!(
            "This profile is meaningless in debug. Run it as:\n\n    \
            cargo run --release --example node_profile\n"
        );
        std::process::exit(1);
    }

    let frame: Vec<u8> = (0..512).map(|i| i as u8).collect();
    let packet = dmx_packet(UNIVERSE, frame.clone());

    println!("# Artnet node cost\n");
    println!(
        "A full {}-channel universe at {ARTNET_RATE} frames per second.\n",
        frame.len()
    );

    let stages = [
        Stage {
            // Every figure below is a handful of clock ticks, so the cost of
            // reading the clock is a meaningful part of it. This row is what an
            // empty timed region measures, and the floor the others sit on.
            name: "measurement floor (empty timed region)",
            timing: time(|| ()),
        },
        Stage {
            name: "parse an ArtDmx packet",
            timing: time(|| ArtCommand::from_buffer(black_box(&packet)).unwrap()),
        },
        Stage {
            name: "write a frame to the offline DMX port",
            timing: {
                let mut port = OfflineDmxPort;
                time(move || port.write(black_box(&frame)).unwrap())
            },
        },
        Stage {
            name: "receive, parse and output (serve_one)",
            timing: time_serve_one(&packet),
        },
    ];

    println!("| stage | median | p99 | one core at {ARTNET_RATE} fps |");
    println!("|---|---:|---:|---:|");
    for stage in &stages {
        println!(
            "| {} | {:.2} us | {:.2} us | {:.4}% |",
            stage.name,
            stage.timing.median_us,
            stage.timing.p99_us,
            stage.timing.core_fraction() * 100.0,
        );
    }

    let serving = stages.last().expect("there is at least one stage").timing;
    println!();
    println!(
        "Serving one universe costs {:.4}% of a core.\n",
        serving.core_fraction() * 100.0,
    );
    println!(
        "The DMX port here is the offline port, so this is the node's own work: the\n\
        socket read, the parse, and the dispatch. A real interface adds a USB write\n\
        of about 518 bytes per frame on top, at whatever its driver costs. The\n\
        figure above is the part a node is answerable for, not the whole write path."
    );
}

/// One measured piece of a node's per-frame work.
struct Stage {
    name: &'static str,
    timing: Timing,
}

/// How long a piece of work took.
#[derive(Clone, Copy)]
struct Timing {
    median_us: f64,
    p99_us: f64,
}

impl Timing {
    /// The share of one core this work occupies when repeated at the artnet rate.
    fn core_fraction(&self) -> f64 {
        self.median_us * ARTNET_RATE / 1e6
    }
}

/// Summarize a set of samples.
fn summarize(mut samples: Vec<Duration>) -> Timing {
    assert!(!samples.is_empty(), "a stage collected no samples");
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize].as_secs_f64() * 1e6;
    Timing {
        median_us: at(0.5),
        p99_us: at(0.99),
    }
}

/// Time a piece of work that needs no setup between iterations.
fn time<T>(mut f: impl FnMut() -> T) -> Timing {
    for _ in 0..WARMUP {
        black_box(f());
    }
    let mut samples = Vec::new();
    let start = Instant::now();
    while start.elapsed() < SAMPLE_WINDOW && samples.len() < MAX_SAMPLES {
        let iteration = Instant::now();
        black_box(f());
        samples.push(iteration.elapsed());
    }
    summarize(samples)
}

/// Time a node serving packets that are already waiting for it.
///
/// Each packet is put on the wire outside the timed region, so what is measured
/// is the node's work on an arrived frame rather than the wait for one.
fn time_serve_one(packet: &[u8]) -> Timing {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("failed to bind the node socket");
    let dest = socket.local_addr().expect("node socket has no address");
    let mut node = ArtnetNode::with_socket(
        socket,
        Box::new(OfflineDmxPort),
        ArtnetNodeConfig {
            port_address: UNIVERSE.try_into().expect("universe out of range"),
            short_name: "profile".into(),
            long_name: "artnet node under profile".into(),
        },
    );
    let controller =
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("failed to bind the controller socket");

    let mut serve = |samples: Option<&mut Vec<Duration>>| {
        send(&controller, packet, dest);
        let iteration = Instant::now();
        node.serve_one().expect("the node failed to serve a frame");
        if let Some(samples) = samples {
            samples.push(iteration.elapsed());
        }
    };

    for _ in 0..WARMUP {
        serve(None);
    }
    let mut samples = Vec::new();
    let start = Instant::now();
    while start.elapsed() < SAMPLE_WINDOW && samples.len() < MAX_SAMPLES {
        serve(Some(&mut samples));
    }
    summarize(samples)
}

/// Put one packet on the wire, waiting out a full send buffer.
fn send(socket: &UdpSocket, packet: &[u8], dest: SocketAddr) {
    while let Err(err) = socket.send_to(packet, dest) {
        if err.kind() != std::io::ErrorKind::WouldBlock {
            panic!("failed to send a packet: {err}");
        }
    }
}

/// An ArtDmx packet addressed to `universe`, as it appears on the wire.
fn dmx_packet(universe: u16, data: Vec<u8>) -> Vec<u8> {
    ArtCommand::Output(Output {
        port_address: universe.try_into().expect("universe out of range"),
        data: data.into(),
        ..Default::default()
    })
    .write_to_buffer()
    .expect("failed to build an ArtDmx packet")
}
