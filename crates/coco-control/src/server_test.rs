use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::client::{ControlClient, ControlError};
use crate::protocol::{Action, Reply, Request};

const POLL: Duration = Duration::from_millis(5);
const DEADLINE: Duration = Duration::from_secs(5);

fn bind(wakes: &Arc<AtomicUsize>) -> ControlServer {
    let wakes = Arc::clone(wakes);
    ControlServer::bind(
        0,
        Arc::new(move || {
            wakes.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap()
}

/// Poll the queue like a frame loop would, until a request arrives.
fn next_incoming(server: &ControlServer) -> Incoming {
    let start = Instant::now();
    loop {
        if let Some(incoming) = server.try_recv() {
            return incoming;
        }
        assert!(start.elapsed() < DEADLINE, "no request arrived");
        std::thread::sleep(POLL);
    }
}

#[test]
fn request_reaches_queue_and_reply_reaches_client() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let server = bind(&wakes);
    let port = server.port();
    let client = std::thread::spawn(move || {
        let mut client = ControlClient::connect(port).unwrap();
        client.call(&Request {
            vm: None,
            action: Action::Wait { fields: 3 },
        })
    });
    let incoming = next_incoming(&server);
    assert_eq!(incoming.request.action, Action::Wait { fields: 3 });
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    incoming.reply(Response::Ok(Reply::Done));
    assert_eq!(client.join().unwrap().unwrap(), Reply::Done);
}

#[test]
fn remote_error_surfaces_as_control_error() {
    let server = bind(&Arc::new(AtomicUsize::new(0)));
    let port = server.port();
    let client = std::thread::spawn(move || {
        let mut client = ControlClient::connect(port).unwrap();
        client.call(&Request {
            vm: Some("ghost".into()),
            action: Action::ListVms,
        })
    });
    next_incoming(&server).reply(Response::Err("no such VM".into()));
    match client.join().unwrap() {
        Err(ControlError::Remote(msg)) => assert_eq!(msg, "no such VM"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn malformed_first_line_closes_connection_without_queueing() {
    let server = bind(&Arc::new(AtomicUsize::new(0)));
    let mut stream = TcpStream::connect(server.addr).unwrap();
    stream
        .write_all(b"POST / HTTP/1.1\r\nHost: x\r\n\r\n{\"cmd\":\"list_vms\"}\n")
        .unwrap();
    stream.set_read_timeout(Some(DEADLINE)).unwrap();
    let mut line = String::new();
    // EOF: the server hung up without answering.
    assert_eq!(BufReader::new(&stream).read_line(&mut line).unwrap(), 0);
    std::thread::sleep(POLL * 4);
    assert!(server.try_recv().is_none());
}

#[test]
fn drop_stops_accepting() {
    let server = bind(&Arc::new(AtomicUsize::new(0)));
    let addr = server.addr;
    drop(server);
    let start = Instant::now();
    while TcpStream::connect(addr).is_ok() {
        assert!(start.elapsed() < DEADLINE, "listener still accepting");
        std::thread::sleep(POLL);
    }
}
