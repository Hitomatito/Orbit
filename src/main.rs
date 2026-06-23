#![allow(dead_code)]

mod adapters;
mod app;
mod db;
mod models;
mod rt;

fn main() {
    let db_path = db::Database::default_path().expect("cannot determine database path");
    let _database = db::Database::open(&db_path).expect("cannot open database");

    let rt = rt::AsyncRuntime::new().expect("failed to create async runtime");
    let orbit = app::OrbitApp::new(rt);
    orbit.run();
}
