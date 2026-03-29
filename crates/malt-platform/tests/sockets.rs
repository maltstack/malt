use malt_platform::sockets::{Transport, TransportError};

#[test]
fn default_local_is_platform_appropriate() {
    let t = Transport::default_local();
    let s = t.to_env_string();
    assert!(!s.is_empty());

    #[cfg(unix)]
    assert!(s.contains("daemon.sock"));
    #[cfg(windows)]
    assert!(s.contains("malt-daemon"));
}

#[test]
fn env_string_roundtrip_tcp() {
    let t = Transport::Tcp("127.0.0.1:4280".parse().unwrap());
    let s = t.to_env_string();
    assert_eq!(s, "tcp://127.0.0.1:4280");

    let t2 = Transport::from_env_string(&s).unwrap();
    assert_eq!(t2.to_env_string(), s);
}

#[cfg(unix)]
#[test]
fn env_string_roundtrip_unix() {
    let t = Transport::UnixSocket("/tmp/test.sock".into());
    let s = t.to_env_string();
    let t2 = Transport::from_env_string(&s).unwrap();
    assert_eq!(t2.to_env_string(), s);
}

#[cfg(windows)]
#[test]
fn env_string_roundtrip_pipe() {
    let t = Transport::NamedPipe(r"\\.\pipe\test-pipe".to_string());
    let s = t.to_env_string();
    let t2 = Transport::from_env_string(&s).unwrap();
    assert_eq!(t2.to_env_string(), s);
}

#[test]
fn invalid_tcp_address() {
    let result = Transport::from_env_string("tcp://not-an-address");
    assert!(matches!(result, Err(TransportError::InvalidAddress(_))));
}
