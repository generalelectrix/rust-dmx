use std::time::Duration;

use rust_dmx::{available_ports, select_port_from};

fn main() {
    let mut ports = available_ports(Some(Duration::from_secs(10))).expect("failed to get ports");
    loop {
        let mut port = select_port_from(&mut ports).expect("failed to open port");
        println!("Opened port: \"{}\"", port);
        port.write(&[0, 1, 2, 3, 4, 5]).unwrap();
    }
}
