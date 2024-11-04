use io::Write;
use std::fmt;
use std::io;
use std::time::Duration;
use thiserror::Error;

mod artnet;
mod enttec;
mod offline;

pub use artnet::ArtnetDmxPort;
pub use enttec::EnttecDmxPort;
pub use offline::OfflineDmxPort;

/// Trait for the general notion of a DMX port.
/// This enables creation of an "offline" port to slot into place if an API requires an output.
#[typetag::serde(tag = "type")]
pub trait DmxPort: fmt::Display {
    /// Return the available ports.  The ports will need to be opened before use.
    fn available_ports(wait: Duration) -> anyhow::Result<PortListing>
    where
        Self: Sized;

    /// Open the port for writing.  Implementations should no-op if this is
    /// called twice rather than returning an error.  Primarily used to re-open
    /// a port that has be deserialized.
    fn open(&mut self) -> Result<(), OpenError>;

    /// Close the port.
    fn close(&mut self);

    /// Write a DMX frame out to the port.  If the frame is smaller than the minimum universe size,
    /// it will be padded with zeros.  If the frame is larger than the maximum universe size, the
    /// values beyond the max size will be ignored.
    fn write(&mut self, frame: &[u8]) -> Result<(), WriteError>;
}

/// A listing of available ports.
type PortListing = Vec<Box<dyn DmxPort>>;

/// Gather up all of the providers and use them to get listings of all ports they have available.
/// Return them as a vector of names plus opener functions.
/// This function does not check whether or not any of the ports are in use already.
///
/// If browse_artnet is Some, poll the network for artnet devices for the provided
/// wait time. If None, do not search for artnet nodes.
pub fn available_ports(browse_artnet: Option<Duration>) -> anyhow::Result<PortListing> {
    let mut ports = Vec::new();
    ports.extend(OfflineDmxPort::available_ports(Duration::ZERO)?);
    ports.extend(EnttecDmxPort::available_ports(Duration::ZERO)?);
    if let Some(wait) = browse_artnet {
        ports.extend(ArtnetDmxPort::available_ports(wait)?);
    }
    Ok(ports)
}

/// Prompt the user to select a port via the command prompt.
///
/// If browse_artnet is Some, poll the network for artnet devices for the provided
/// wait time. If None, do not search for artnet nodes.
pub fn select_port(browse_artnet: Option<Duration>) -> anyhow::Result<Box<dyn DmxPort>> {
    let mut ports = available_ports(browse_artnet)?;
    println!("Available DMX ports:");
    for (i, port) in ports.iter().enumerate() {
        println!("{}: {}", i, port);
    }
    let mut port = loop {
        print!("Select a port: ");
        io::stdout().flush()?;
        let input = read_string()?;
        let index = match input.trim().parse::<usize>() {
            Ok(num) => num,
            Err(e) => {
                println!("{}; please enter an integer.", e);
                continue;
            }
        };
        if index >= ports.len() {
            println!("Please enter a value less than {}.", ports.len());
            continue;
        }
        break ports.swap_remove(index);
    };
    port.open()?;
    Ok(port)
}

/// Read a line of input from stdin.
fn read_string() -> Result<String, io::Error> {
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

#[derive(Error, Debug)]
pub enum OpenError {
    #[error("the DMX port is not connected")]
    NotConnected,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Error, Debug)]
pub enum WriteError {
    #[error("the DMX port is not connected")]
    Disconnected,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
