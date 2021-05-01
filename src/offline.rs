use crate::{DmxPort, Error, PortOpener};

use std::fmt;

#[derive(Debug)]
pub struct OfflineDmxPort;

impl DmxPort for OfflineDmxPort {
    fn available_ports() -> Vec<(String, Box<PortOpener>)> {
        vec![("offline".to_string(), Box::new(|| Ok(Box::new(Self))))]
    }

    fn write(&mut self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
}

impl fmt::Display for OfflineDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "offline")
    }
}
