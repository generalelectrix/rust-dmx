use derive_more::Display;
use serialport::{Error as SerialError, SerialPortInfo};
use std::error::Error as StdError;
use std::fmt;

mod enttec;
mod offline;

pub use enttec::EnttecDmxPort;
pub use offline::OfflineDmxPort;

/// Trait for the general notion of a DMX port.
/// This enables creation of an "offline" port to slot into place if an API requires an output.
#[typetag::serde(tag = "type")]
pub trait DmxPort: fmt::Display {
    /// Return the available ports, and closures that can open them.
    fn available_ports() -> Result<PortListing, Error>
    where
        Self: Sized;

    /// Open the port for writing.  Implementations should no-op if this is
    /// called twice rather than returning an error.  Primarily used to re-open
    /// a port that has be deserialized.
    fn open(&mut self) -> Result<(), Error>;

    /// Close the port.
    fn close(&mut self);

    /// Write a DMX frame out to the port.  If the frame is smaller than the minimum universe size,
    /// it will be padded with zeros.  If the frame is larger than the maximum universe size, the
    /// values beyond the max size will be ignored.
    fn write(&mut self, frame: &[u8]) -> Result<(), Error>;
}

/// A closure that returns a fully-opened port.
pub type PortOpener = dyn Fn() -> Result<Box<dyn DmxPort>, Error>;

/// A listing of available ports, with opener functions.
type PortListing = Vec<(SerialPortInfo, Box<PortOpener>)>;

/// Gather up all of the providers and use them to get listings of all ports they have available.
/// Return them as a vector of names plus opener functions.
/// This function does not check whether or not any of the ports are in use already.
pub fn available_ports() -> Result<PortListing, Error> {
    let mut ports = Vec::new();
    ports.extend(OfflineDmxPort::available_ports()?.into_iter());
    ports.extend(EnttecDmxPort::available_ports()?.into_iter());
    Ok(ports)
}

#[derive(Debug, Display)]
pub enum Error {
    Serial(SerialError),
    IO(std::io::Error),
    PortClosed,
}

impl From<SerialError> for Error {
    fn from(e: SerialError) -> Self {
        Error::Serial(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IO(e)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        use Error::*;
        match *self {
            Serial(ref e) => Some(e),
            IO(ref e) => Some(e),
            PortClosed => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test() {
        let (_, open) = available_ports().unwrap().pop().unwrap();
        let mut port = open().unwrap();
        let frame = [255, 255, 255];
        port.write(&frame[..]).unwrap();
        loop {}
    }
}
