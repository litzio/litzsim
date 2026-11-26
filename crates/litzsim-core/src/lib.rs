use serde_json::{Value, json};

pub mod circuit;
pub mod layout;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn ping() -> Value {
    json!({ "ok": true, "version": VERSION })
}
