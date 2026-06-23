#![allow(dead_code)]

mod adapters;
mod app;
mod models;

fn main() {
    let orbit = app::OrbitApp::new();
    orbit.run();
}
