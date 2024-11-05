use std::time::Duration;

use rust_dmx::select_port;

fn main() {
    let mut port = select_port(Some(Duration::from_secs(1))).expect("failed to open port");
    println!("Opened port: \"{}\"", port);
    port.write(vec![0, 1, 2, 3, 4, 5].as_slice()).unwrap();
}
