#![allow(dead_code)]

mod adapters;
mod app;
mod models;
mod rt;

fn main() {
    let rt = rt::AsyncRuntime::new().expect("failed to create async runtime");
    let orbit = app::OrbitApp::new(rt);
    orbit.run();
}
