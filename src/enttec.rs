//! Implementation of support for the Enttec USB DMX Pro dongle.
use regex::Regex;
use std::cmp::min;
use std::fmt;
use std::fs;
use std::io::Write;
use std::time::Duration;

use super::{DmxPort, DmxPortProvider, Error};
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

/// Format a byte buffer as an enttec message.
/// Maximum valid size for payload is 600; no check is made here that the payload is within this range.
fn make_packet(message_type: u8, payload: &[u8]) -> Vec<u8> {
    // Enttec messages are the size of the payload plus 5 bytes for type, length, and framing.
    let payload_size = payload.len();
    let mut packet = Vec::with_capacity(payload_size + 5);
    let (len_lsb, len_msb) = (payload_size as u8, (payload_size >> 8) as u8);
    packet.push(START_VAL);
    packet.push(message_type);
    packet.push(len_lsb);
    packet.push(len_msb);
    packet.extend_from_slice(payload);
    packet.push(END_VAL);
    packet
}

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
    fn as_packet(&self) -> Vec<u8> {
        let payload = [
            self.break_time,
            self.mark_after_break_time,
            self.output_rate,
        ];
        make_packet(SET_PARAMETERS, &payload)
    }
}

pub struct EnttecDmxPort {
    params: EnttecParams,
    port: SystemPort,
    port_name: String,
}

pub const ENTTEC_NAMESPACE: &'static str = "enttec";

impl EnttecDmxPort {
    /// Open a enttec port with the specified port name.
    pub fn new<N: Into<String>>(port_name: N) -> Result<EnttecDmxPort, Error> {
        let port_name = port_name.into();
        let port_path = format!("{}{}", ENTTEC_PATH_PREFIX, port_name);
        let mut port = open(&port_path)?;

        // use a short 1 ms timeout to avoid blocking if, say, the port disappears
        port.set_timeout(Duration::from_millis(1))?;

        let params = EnttecParams::default();

        let mut port = EnttecDmxPort {
            params: params,
            port: port,
            port_name: port_name,
        };

        // send the default parameters to the port
        port.write_params()?;
        Ok(port)
    }

    /// Write the current parameters out to the port.
    pub fn write_params(&mut self) -> Result<(), Error> {
        self.port.write_all(&self.params.as_packet())?;
        Ok(())
    }
}

impl fmt::Debug for EnttecDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.serializable().fmt(f)
    }
}

impl DmxPort for EnttecDmxPort {
    fn write(&mut self, frame: &[u8]) -> Result<(), Error> {
        let packet = {
            let size = frame.len();
            if size < MIN_UNIVERSE_SIZE {
                //
                let mut padded_frame = Vec::with_capacity(MIN_UNIVERSE_SIZE);
                padded_frame.extend_from_slice(frame);
                padded_frame.resize(MIN_UNIVERSE_SIZE, 0);
                make_packet(SEND_DMX_PACKET, &padded_frame)
            } else {
                make_packet(SEND_DMX_PACKET, &frame[0..min(size, MAX_UNIVERSE_SIZE)])
            }
        };
        self.port.write_all(&packet)?;
        Ok(())
    }

    fn namespace(&self) -> &str {
        ENTTEC_NAMESPACE
    }

    fn port_name(&self) -> &str {
        &self.port_name
    }
}

pub struct EnttecPortProvider;

const ENTTEC_PATH_PREFIX: &'static str = "/dev/tty.usbserial-";

impl DmxPortProvider for EnttecPortProvider {
    /// Return a unique namespace for this port provider.
    fn namespace(&self) -> &str {
        ENTTEC_NAMESPACE
    }
    /// Return the available enttec ports connected to this system.
    /// TODO: provide a mechanism to specialize this implementation depending on platform.
    fn available_ports(&self) -> Vec<String> {
        match fs::read_dir("/dev/") {
            Err(_) => Vec::new(),
            Ok(dirs) => dirs
                .filter_map(|x| x.ok())
                .filter_map(|p| {
                    p.path().to_str().and_then(|p| {
                        if p.starts_with(ENTTEC_PATH_PREFIX) {
                            let port_name = p[ENTTEC_PATH_PREFIX.len()..].to_string();
                            Some(port_name)
                        } else {
                            None
                        }
                    })
                })
                .collect(),
        }
    }
    /// Attempt to open this port, and return it behind the trait object or an error.
    fn open<N: Into<String>>(&self, port: N) -> Result<Box<DmxPort>, Error> {
        EnttecDmxPort::new(port).map(|p| Box::new(p) as Box<DmxPort>)
    }
}
