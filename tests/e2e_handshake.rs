//! End-to-end JSON-RPC handshake test. Spawns the built binary,
//! drives the `initialize` round-trip over stdin/stdout, and
//! asserts the reply advertises the four `google_*` tool defs.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_nexo-plugin-google")
}

#[test]
fn initialize_reply_lists_four_google_tools() {
    let mut child = Command::new(bin_path())
        .env("RUST_LOG", "warn")
        // No NEXO_BROKER_URL → plugin skips broker subscriber spawn
        // but still serves JSON-RPC on stdio.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn binary");

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);

    // Send initialize JSON-RPC frame.
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {},
    });
    let mut payload = serde_json::to_vec(&request).expect("serialise");
    payload.push(b'\n');
    stdin.write_all(&payload).expect("write request");
    stdin.flush().expect("flush request");

    // Read reply with a small deadline so a hang surfaces clearly.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        let n = reader.read_line(&mut line).expect("read reply");
        if n == 0 {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Got something — parse + assert.
        let parsed: Value = serde_json::from_str(trimmed)
            .unwrap_or_else(|e| panic!("non-JSON reply line: {trimmed} ({e})"));
        assert_eq!(parsed["id"], serde_json::json!(1));
        let tools = parsed["result"]["tools"]
            .as_array()
            .expect("initialize reply must include `tools` array");
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(
            names,
            vec![
                "google_auth_start",
                "google_auth_status",
                "google_call",
                "google_auth_revoke",
            ]
        );
        // Sanity-check the manifest section round-tripped too.
        let manifest = &parsed["result"]["manifest"];
        assert_eq!(manifest["plugin"]["id"], serde_json::json!("google"));
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("initialize reply not received within 15s");
}
