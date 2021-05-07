use crate::{DmxPort, Error, PortListing};
use serde::{Deserialize, Serialize};

use std::fmt;

#[derive(Debug, Serialize, Deserialize)]
pub struct OfflineDmxPort;

#[typetag::serde]
impl DmxPort for OfflineDmxPort {
    fn available_ports() -> Result<PortListing, Error> {
        Ok(vec![(Box::new(Self))])
    }

    fn open(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn close(&mut self) {}

    fn write(&mut self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
}

impl fmt::Display for OfflineDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "offline")
    }
}
