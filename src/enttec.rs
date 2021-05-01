//! Implementation of support for the Enttec USB DMX Pro dongle.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fs;
use std::io::Write;
use std::time::Duration;
use std::{cmp::min, fmt};

use crate::{PortListing, PortOpener};

use super::{DmxPort, Error};
use serial::prelude::*;
use serial::{open, SystemPort};

// Some constants used for enttec message framing.
const START_VAL: u8 = 0x7E;
const END_VAL: u8 = 0xE7;

// Universe size constraints.
const MIN_UNIVERSE_SIZE: usize = 24;
const MAX_UNIVERSE_SIZE: usize = 512;

// Port action flags.
const SET_PARAMETERS: u8 = 4;
//const RECEIVE_DMX_PACKET: u8 = 5;
const SEND_DMX_PACKET: u8 = 6;

/// Format a byte buffer as an enttec message into the provided output buffer.
/// Maximum valid size for payload is 600; no check is made here that the payload is within this range.
fn make_packet(message_type: u8, payload: &[u8], output: &mut Vec<u8>) {
    // Enttec messages are the size of the payload plus 5 bytes for type, length, and framing.
    let payload_size = payload.len();
    output.clear();
    output.reserve(payload_size + 5);
    let (len_lsb, len_msb) = (payload_size as u8, (payload_size >> 8) as u8);
    output.push(START_VAL);
    output.push(message_type);
    output.push(len_lsb);
    output.push(len_msb);
    output.extend_from_slice(payload);
    output.push(END_VAL);
}

#[derive(Debug)]
pub struct EnttecParams {
    /// DMX output break time in 10.67 microsecond units. Valid range is 9 to 127.
    break_time: u8,
    /// DMX output Mark After Break time in 10.67 microsecond units. Valid range is 1 to 127.
    mark_after_break_time: u8,
    /// DMX output rate in packets per second. Valid range is 1 to 40, or 0 for fastest rate
    /// possible (this will make the most difference when the output universe size is smallest).
    output_rate: u8,
}

impl Default for EnttecParams {
    /// Default parameters for the enttec port.
    /// In summary: minimum break and mark times, fastest fixed framerate.
    fn default() -> Self {
        EnttecParams {
            break_time: 9,
            mark_after_break_time: 1,
            output_rate: 40,
        }
    }
}

impl EnttecParams {
    fn as_packet(&self, output: &mut Vec<u8>) {
        let payload = [
            self.break_time,
            self.mark_after_break_time,
            self.output_rate,
        ];
        make_packet(SET_PARAMETERS, &payload, output)
    }
}

pub struct EnttecDmxPort {
    params: EnttecParams,
    port: Option<SystemPort>,
    port_name: String,
    output_buffer: Vec<u8>,
}

impl EnttecDmxPort {
    /// Create an enttec port with the specified port name.
    /// The port is not opened yet.
    pub fn new<N: Into<String>>(port_name: N) -> Self {
        let port_name = port_name.into();

        let params = EnttecParams::default();

        Self {
            params: params,
            port: None,
            port_name: port_name,
            output_buffer: Vec::new(),
        }
    }

    /// Create an enttec port with the specified port name, and open it.
    pub fn opened<N: Into<String>>(port_name: N) -> Result<Self, Error> {
        let mut port = Self::new(port_name);
        port.open()?;
        Ok(port)
    }

    /// Write a packet to the port.
    /// If the port is not open, return an error.

    /// Write the current parameters out to the port.
    fn write_params(&mut self) -> Result<(), Error> {
        self.params.as_packet(&mut self.output_buffer);
        self.write_output_buffer()
    }

    fn write_output_buffer(&mut self) -> Result<(), Error> {
        match &mut self.port {
            Some(p) => p.write_all(&self.output_buffer)?,
            None => return Err(Error::PortClosed),
        }
        Ok(())
    }
}

#[typetag::serde]
impl DmxPort for EnttecDmxPort {
    /// Return the available enttec ports connected to this system.
    /// TODO: provide a mechanism to specialize this implementation depending on platform.
    fn available_ports() -> PortListing {
        match fs::read_dir("/dev/") {
            Err(_) => Vec::new(),
            Ok(dirs) => dirs
                .filter_map(|x| x.ok())
                .filter_map(|p| {
                    p.path().to_str().and_then(|p| {
                        if p.starts_with(ENTTEC_PATH_PREFIX) {
                            let port_name = p[ENTTEC_PATH_PREFIX.len()..].to_string();
                            let open_name = port_name.clone();
                            let opener: Box<PortOpener> = Box::new(move || {
                                EnttecDmxPort::opened(open_name.clone())
                                    .map(|p| Box::new(p) as Box<dyn DmxPort>)
                            });
                            Some((port_name, opener))
                        } else {
                            None
                        }
                    })
                })
                .collect(),
        }
    }

    /// Open the port.
    fn open(&mut self) -> Result<(), Error> {
        if self.port.is_some() {
            return Ok(());
        }

        let port_path = format!("{}{}", ENTTEC_PATH_PREFIX, self.port_name);
        let mut port = open(&port_path)?;

        // use a short 1 ms timeout to avoid blocking if, say, the port disappears
        port.set_timeout(Duration::from_millis(1))?;

        // send the default parameters to the port
        self.write_params()?;

        self.port = Some(port);
        Ok(())
    }

    fn close(&mut self) {
        self.port = None;
    }

    fn write(&mut self, frame: &[u8]) -> Result<(), Error> {
        let size = frame.len();
        if size < MIN_UNIVERSE_SIZE {
            let mut padded_frame = Vec::with_capacity(MIN_UNIVERSE_SIZE);
            padded_frame.extend_from_slice(frame);
            padded_frame.resize(MIN_UNIVERSE_SIZE, 0);
            make_packet(SEND_DMX_PACKET, &padded_frame, &mut self.output_buffer)
        } else {
            make_packet(
                SEND_DMX_PACKET,
                &frame[0..min(size, MAX_UNIVERSE_SIZE)],
                &mut self.output_buffer,
            )
        }

        self.write_output_buffer()
    }
}

impl Serialize for EnttecDmxPort {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.port_name.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EnttecDmxPort {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

const ENTTEC_PATH_PREFIX: &'static str = "/dev/tty.usbserial-";

impl fmt::Display for EnttecDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.port_name)
    }
}
