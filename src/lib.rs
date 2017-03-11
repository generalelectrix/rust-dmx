extern crate serial;

pub use serial::Error;

mod enttec;

pub use enttec::{available_enttec_ports, EnttecDmxPort};

/// Trait for the general notion of a DMX port.
/// This enables creation of an "offline" port to slot into place if an API requires an output.
pub trait DmxPort {
    /// Write a DMX frame out to the port.  If the frame is smaller than the minimum universe size,
    /// it will be padded with zeros.  If the frame is larger than the maximum universe size, the
    /// values beyond the max size will be ignored.
    fn write(&mut self, frame: &[u8]) -> Result<(), Error>;

    fn port_name(&self) -> &str;
}

pub struct OfflineDmxPort {}

impl DmxPort for OfflineDmxPort {
    fn write(&mut self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn port_name(&self) -> &str {
        "offline"
    }
}