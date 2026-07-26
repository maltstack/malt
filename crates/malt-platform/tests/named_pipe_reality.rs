#![cfg(windows)]

use std::io::{Read, Write};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use malt_platform::ipc::{current_process_principal, NamedPipeClient, NamedPipeServer};

#[test]
fn named_pipe_accepts_a_real_client_and_attributes_its_process() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let name = format!("malt-platform-test-{}-{suffix}", std::process::id());
    let server_name = name.clone();

    let expected_principal = current_process_principal().expect("read current process principal");
    let server = thread::spawn(move || {
        let server = NamedPipeServer::create(&server_name).expect("create named pipe");
        let mut connection = server.accept().expect("accept named pipe client");
        let identity = connection
            .peer_identity()
            .expect("query client process identity");
        let mut request = [0u8; 4];
        connection
            .file()
            .read_exact(&mut request)
            .expect("read request");
        connection
            .file()
            .write_all(b"pong")
            .expect("write response");
        identity
    });

    let mut client = loop {
        match NamedPipeClient::connect(&name) {
            Ok(client) => break client,
            Err(error) if error.raw_os_error() == Some(2) => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("connect named pipe: {error}"),
        }
    };
    client.file().write_all(b"ping").expect("write request");
    let mut response = [0u8; 4];
    client
        .file()
        .read_exact(&mut response)
        .expect("read response");

    assert_eq!(&response, b"pong");
    let identity = server.join().expect("server thread");
    assert_eq!(identity.process_id, std::process::id());
    assert_eq!(identity.principal, expected_principal);
}
