use malt_protocol::framing::{Frame, FrameError, FrameFlags, FrameReader, FrameWriter};
use std::io::Cursor;

#[test]
fn roundtrip_empty_payload() {
    let frame = Frame { flags: FrameFlags::new(), payload: vec![] };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();
    let decoded = FrameReader::new(Cursor::new(&buf)).read_frame().unwrap();
    assert_eq!(frame.payload, decoded.payload);
    assert_eq!(frame.flags, decoded.flags);
}

#[test]
fn roundtrip_with_payload() {
    let frame = Frame { flags: FrameFlags::new(), payload: vec![0xDE, 0xAD, 0xBE, 0xEF] };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();
    let decoded = FrameReader::new(Cursor::new(&buf)).read_frame().unwrap();
    assert_eq!(frame.payload, decoded.payload);
}

#[test]
fn roundtrip_all_flags() {
    let mut flags = FrameFlags::new();
    flags.set_compressed(true);
    flags.set_json_encoded(true);
    flags.set_continuation(true);
    let frame = Frame { flags, payload: vec![1, 2, 3] };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();
    let decoded = FrameReader::new(Cursor::new(&buf)).read_frame().unwrap();
    assert!(decoded.flags.compressed());
    assert!(decoded.flags.json_encoded());
    assert!(decoded.flags.continuation());
}

#[test]
fn flags_individual_bits() {
    let mut f = FrameFlags::new();
    assert!(!f.compressed());
    assert!(!f.json_encoded());
    assert!(!f.continuation());
    f.set_compressed(true);
    assert!(f.compressed());
    assert!(!f.json_encoded());
    f.set_json_encoded(true);
    assert!(f.json_encoded());
    f.set_continuation(true);
    assert!(f.continuation());
}

#[test]
fn reject_frame_too_large() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(1_048_576u32).to_le_bytes());
    buf.push(0x00);
    buf.extend(vec![0u8; 1_048_576]);
    let result = FrameReader::with_max_frame_size(Cursor::new(&buf), 65_536).read_frame();
    assert!(matches!(result, Err(FrameError::FrameTooLarge { .. })));
}

#[test]
fn reject_truncated_frame() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&100u32.to_le_bytes());
    buf.push(0x00);
    buf.extend(vec![0u8; 5]);
    let result = FrameReader::new(Cursor::new(&buf)).read_frame();
    assert!(matches!(result, Err(FrameError::UnexpectedEof)));
}

#[test]
fn reject_reserved_flags() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.push(0b1000_0000);
    buf.push(0xFF);
    let result = FrameReader::new(Cursor::new(&buf)).read_frame();
    assert!(matches!(result, Err(FrameError::ReservedFlagsSet(_))));
}

#[test]
fn wire_format_is_length_flags_payload() {
    let frame = Frame { flags: FrameFlags::new(), payload: vec![0xAA, 0xBB] };
    let mut buf = Vec::new();
    FrameWriter::new(&mut buf).write_frame(&frame).unwrap();
    assert_eq!(buf, vec![2, 0, 0, 0, 0x00, 0xAA, 0xBB]);
}

#[test]
fn multiple_frames_sequential() {
    let frames = vec![
        Frame { flags: FrameFlags::new(), payload: vec![1] },
        Frame { flags: FrameFlags::new(), payload: vec![2, 3] },
        Frame { flags: FrameFlags::new(), payload: vec![4, 5, 6] },
    ];
    let mut buf = Vec::new();
    let mut writer = FrameWriter::new(&mut buf);
    for f in &frames { writer.write_frame(f).unwrap(); }
    let mut reader = FrameReader::new(Cursor::new(&buf));
    for expected in &frames {
        let decoded = reader.read_frame().unwrap();
        assert_eq!(expected.payload, decoded.payload);
    }
}
