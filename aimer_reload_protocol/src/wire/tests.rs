use std::io::Cursor;
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use super::*;

#[test]
fn module_transfer_accepts_every_chunk_boundary_without_buffering_one_module_frame() {
    let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
    let limits = ProtocolLimits::new(1024, Duration::from_secs(1)).max_chunk_bytes(3);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server_credentials = credentials.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        receive_module_and_acknowledge(&mut stream, &server_credentials, limits, |module| {
            assert_eq!(module, b"0123456789");
            Ok(())
        })
        .unwrap()
    });
    let mut stream = TcpStream::connect(address).unwrap();

    let acknowledgement = send_module(&mut stream, &credentials, limits, 7, b"0123456789").unwrap();

    assert_eq!(acknowledgement.module_len, 10);
    assert_eq!(server.join().unwrap(), acknowledgement);
}

#[test]
fn authenticated_frame_has_a_stable_golden_vector() {
    let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
    let transcript = transcript(credentials.session_id(), &[0x22; 32], &[0x33; 32]);
    let keys = derive_keys(&credentials, &transcript).unwrap();
    let mut bytes = Vec::new();

    write_frame(
        &mut bytes,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_END,
        0x0102_0304_0506_0708,
        9,
        b"golden",
    )
    .unwrap();

    assert_eq!(
        hex::encode(bytes),
        "414d524c01000000030000005800000016000000000000001111111111111111111111111111111108070605040302010900000000000000f337a33a670f7f15e10cb873eebd2dbbf317092ae625c252b18fcbf7db09c45b3df6e3e86c01071b83f4be4c1702f466469b8279b760"
    );
}

#[test]
fn every_frame_truncation_and_authenticated_header_mutation_is_rejected() {
    let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
    let transcript = transcript(credentials.session_id(), &[0x22; 32], &[0x33; 32]);
    let keys = derive_keys(&credentials, &transcript).unwrap();
    let mut valid = Vec::new();
    write_frame(
        &mut valid,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_END,
        7,
        2,
        b"frame",
    )
    .unwrap();

    for end in 0..valid.len() {
        assert!(
            read_frame(
                &mut Cursor::new(&valid[..end]),
                &keys.client_to_server,
                credentials.session_id(),
                5,
            )
            .is_err()
        );
    }
    for index in [0, 4, 6, 10, 12, 14, 24, 56, valid.len() - 1] {
        let mut malformed = valid.clone();
        malformed[index] ^= 1;
        assert!(
            read_frame(
                &mut Cursor::new(malformed),
                &keys.client_to_server,
                credentials.session_id(),
                5,
            )
            .is_err()
        );
    }
}

#[test]
fn client_rejects_an_incompatible_handshake_minor_before_authentication() {
    let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
    let mut challenge = Vec::new();
    challenge.extend_from_slice(HANDSHAKE_MAGIC);
    challenge.extend_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    challenge.extend_from_slice(&(PROTOCOL_MINOR + 1).to_le_bytes());
    challenge.extend_from_slice(&[0x22; 32]);
    let mut stream = Cursor::new(challenge);

    assert!(matches!(
        client_handshake(&mut stream, &credentials),
        Err(ProtocolError::Authentication)
    ));
}

#[test]
fn authenticated_chunk_duplicates_gaps_and_overlaps_are_rejected() {
    let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
    let limits = ProtocolLimits::new(6, Duration::from_secs(1)).max_chunk_bytes(3);

    for (second_sequence, second_offset) in [(1_u64, 3_u64), (3, 3), (2, 2), (2, 4)] {
        let transcript = transcript(credentials.session_id(), &[0x22; 32], &[0x33; 32]);
        let keys = derive_keys(&credentials, &transcript).unwrap();
        let mut frames = Vec::new();
        write_frame(
            &mut frames,
            &keys.client_to_server,
            credentials.session_id(),
            KIND_MODULE_CHUNK,
            7,
            1,
            &[&0_u64.to_le_bytes()[..], b"abc"].concat(),
        )
        .unwrap();
        write_frame(
            &mut frames,
            &keys.client_to_server,
            credentials.session_id(),
            KIND_MODULE_CHUNK,
            7,
            second_sequence,
            &[&second_offset.to_le_bytes()[..], b"def"].concat(),
        )
        .unwrap();

        assert!(
            read_module_chunks(
                &mut Cursor::new(frames),
                &keys.client_to_server,
                credentials.session_id(),
                limits,
                7,
                6,
            )
            .is_err()
        );
    }
}

#[test]
fn reconnect_transcripts_derive_separate_keys_and_reject_replayed_frames() {
    let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
    let first_transcript = transcript(credentials.session_id(), &[0x22; 32], &[0x33; 32]);
    let second_transcript = transcript(credentials.session_id(), &[0x44; 32], &[0x55; 32]);
    let first_keys = derive_keys(&credentials, &first_transcript).unwrap();
    let second_keys = derive_keys(&credentials, &second_transcript).unwrap();
    let mut first_frame = Vec::new();
    let mut second_frame = Vec::new();
    write_frame(
        &mut first_frame,
        &first_keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_END,
        7,
        1,
        b"same",
    )
    .unwrap();
    write_frame(
        &mut second_frame,
        &second_keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_END,
        7,
        1,
        b"same",
    )
    .unwrap();

    assert_ne!(first_frame, second_frame);
    assert!(matches!(
        read_frame(
            &mut Cursor::new(first_frame),
            &second_keys.client_to_server,
            credentials.session_id(),
            4,
        ),
        Err(ProtocolError::Authentication)
    ));
}
