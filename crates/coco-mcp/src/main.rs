//! `cocovm-mcp`: an MCP server over stdio that exposes a running `cocovm`
//! app's control protocol ([`coco_control`]) as tools. stdout carries only
//! the JSON-RPC protocol; every diagnostic goes to stderr.

mod backend;
mod jsonrpc;
mod mcp;
mod tool_defs;
mod tools;

use std::io::{self, BufReader};

use clap::Parser;

use backend::AppBackend;
use mcp::Mcp;

/// MCP server (stdio) that lets an AI drive a running cocovm VM.
#[derive(Parser)]
#[command(name = "cocovm-mcp", version)]
struct Cli {
    /// Control port of the running cocovm app.
    #[arg(long, env = coco_control::PORT_ENV, default_value_t = coco_control::DEFAULT_PORT)]
    port: u16,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    eprintln!("cocovm-mcp: targeting control port {}", cli.port);

    let backend = Box::new(AppBackend::new(cli.port));
    let mut handler = Mcp::new(backend, cli.port);

    let stdin = io::stdin();
    let stdout = io::stdout();
    let result = jsonrpc::run(BufReader::new(stdin.lock()), stdout.lock(), &mut handler);
    if let Err(e) = &result {
        eprintln!("cocovm-mcp: {e}");
    }
    result
}
