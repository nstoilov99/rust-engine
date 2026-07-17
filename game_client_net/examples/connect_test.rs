//! Package 2 acceptance harness. Run against a local standalone:
//!   cargo run -p game_client_net --example connect_test
//!   cargo run -p game_client_net --example connect_test -- --pretend-version 999
//! Exits 0 on the expected outcome (InWorld, or a clean VersionMismatch when
//! pretending), 1 otherwise. Prints the identity so token persistence can be
//! checked across runs.

use std::time::{Duration, Instant};

use game_client_net::SpacetimeNetClient;
use game_shared::net::protocol::ModuleAddr;
use game_shared::net::traits::{ConnectionState, DisconnectReason, NetClient, NetEvent};

fn main() {
    let pretend_version = std::env::args()
        .skip_while(|a| a != "--pretend-version")
        .nth(1)
        .map(|v| v.parse::<u32>().expect("--pretend-version takes a u32"));
    // --hold: stay InWorld and wait for the server to die; PASS on a clean
    // ConnectionLost (no panic). Used for the kill-server acceptance test.
    let hold = std::env::args().any(|a| a == "--hold");

    let mut client = SpacetimeNetClient::new();
    if let Some(v) = pretend_version {
        client.set_client_version(v);
        println!("pretending client protocol version {v}");
    }

    client.connect(&ModuleAddr {
        host: "http://127.0.0.1:3000".to_string(),
        module: "rust-engine-dev".to_string(),
    });

    let deadline = Instant::now() + Duration::from_secs(if hold { 60 } else { 10 });
    let mut events = Vec::new();
    loop {
        client.poll(&mut events);
        for ev in events.drain(..) {
            println!("event: {ev:?}");
            match ev {
                NetEvent::Connected => {
                    println!(
                        "state: {:?}, identity: {}",
                        client.connection_state(),
                        client.identity_hex().unwrap_or_default()
                    );
                    if pretend_version.is_some() {
                        eprintln!("FAIL: expected version mismatch, got Connected");
                        std::process::exit(1);
                    }
                    if !hold {
                        println!("PASS: reached InWorld");
                        std::process::exit(0);
                    }
                    println!("holding InWorld; kill the server now (60 s window)");
                }
                NetEvent::Disconnected(DisconnectReason::VersionMismatch { server, client: c }) => {
                    if pretend_version.is_some() {
                        println!("PASS: clean version mismatch (server {server}, client {c})");
                        std::process::exit(0);
                    }
                    eprintln!("FAIL: unexpected version mismatch");
                    std::process::exit(1);
                }
                NetEvent::Disconnected(DisconnectReason::ConnectionLost(msg)) if hold => {
                    println!("PASS: clean ConnectionLost (\"{msg}\"), no panic");
                    std::process::exit(0);
                }
                NetEvent::Disconnected(reason) => {
                    eprintln!("FAIL: disconnected: {reason:?}");
                    std::process::exit(1);
                }
                _ => {}
            }
        }
        if Instant::now() > deadline {
            eprintln!(
                "FAIL: timed out in state {:?}",
                client.connection_state()
            );
            std::process::exit(1);
        }
        assert!(
            client.connection_state() != ConnectionState::Offline,
            "client never left Offline"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
}
