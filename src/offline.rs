use crate::{DmxPort, OpenError, PortListing, WriteError};
use serde::{Deserialize, Serialize};

use std::{fmt, time::Duration};

#[derive(Debug, Serialize, Deserialize)]
pub struct OfflineDmxPort;

#[typetag::serde]
impl DmxPort for OfflineDmxPort {
    fn available_ports(_: Duration) -> anyhow::Result<PortListing> {
        Ok(vec![(Box::new(Self))])
    }

    fn open(&mut self) -> Result<(), OpenError> {
        Ok(())
    }

    fn close(&mut self) {}

    fn write(&mut self, _: &[u8]) -> Result<(), WriteError> {
        Ok(())
    }
}

impl fmt::Display for OfflineDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "offline")
    }
}
