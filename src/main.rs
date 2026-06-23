#![allow(dead_code)]

mod adapters;
mod models;

fn main() {
    println!("Orbit v{}", env!("CARGO_PKG_VERSION"));
}
