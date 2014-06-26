#![crate_id = "enttec#1"]
#![deny(missing_doc)]

//! A library to support interfacing with a Enttec Pro USB DMX port on Mac OS X.

#![feature(globs)]
#![feature(phase)]

#[phase(plugin, link)] extern crate log;

extern crate native;
extern crate libc;
extern crate rustrt;

pub mod enttec_pro_port;
mod posix_port;