use std::io::{Read, Write};

use ring::aead;
use ring::hkdf;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use super::{
    ModuleMetadata, ProtocolError, ProtocolLimits, ReloadCommand, ReloadResult, SessionCredentials,
    TransferAcknowledgement,
};

mod frame;

use frame::*;

const PROTOCOL_MAJOR: u16 = 1;
const PROTOCOL_MINOR: u16 = 0;
const HANDSHAKE_MAGIC: &[u8; 4] = b"AMRH";
const FRAME_MAGIC: &[u8; 4] = b"AMRL";
const CHALLENGE_LEN: usize = 40;
const CLIENT_AUTH_LEN: usize = 88;
const SERVER_AUTH_LEN: usize = 40;
const FRAME_HEADER_LEN: usize = 88;
const AUTH_TAG_START: usize = 56;
const AUTH_TAG_END: usize = 88;
const KIND_MODULE_BEGIN: u16 = 1;
const KIND_MODULE_CHUNK: u16 = 2;
const KIND_MODULE_END: u16 = 3;
const KIND_UPLOAD_ACCEPTED: u16 = 4;
const KIND_RELOAD_BEGIN: u16 = 5;
const KIND_RELOAD_RESULT: u16 = 6;
const KIND_RESULT_QUERY: u16 = 7;
const KIND_RESULT_UNAVAILABLE: u16 = 8;
const RELOAD_BEGIN_LEN: usize = 108;

#[derive(Clone, Copy)]
struct FrameKeyLength;

impl hkdf::KeyType for FrameKeyLength {
    fn len(&self) -> usize {
        32
    }
}

struct DirectionalKeys {
    client_to_server: FrameKeys,
    server_to_client: FrameKeys,
}

struct FrameKeys {
    authentication: hmac::Key,
    encryption: aead::LessSafeKey,
}

struct Frame {
    kind: u16,
    request_id: u64,
    sequence: u64,
    payload: Vec<u8>,
}

/// The authenticated operation served by one accepted connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReloadConnectionOutcome {
    /// A complete command executed or recovered its idempotent result.
    Command(ReloadResult),
    /// A reconnect query recovered a terminal result, or found none retained.
    Query(Option<ReloadResult>),
}

/// Authenticates and sends one complete module over an ordered byte stream.
pub fn send_module<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    request_id: u64,
    module: &[u8],
) -> Result<TransferAcknowledgement, ProtocolError>
where
    S: Read + Write,
{
    if module.len() > limits.max_module_bytes() {
        return Err(ProtocolError::ModuleTooLarge {
            actual: module.len(),
            maximum: limits.max_module_bytes(),
        });
    }
    if !module.is_empty() && limits.chunk_bytes_limit() == 0 {
        return Err(ProtocolError::InvalidFrame(
            "non-empty module requires a nonzero chunk limit",
        ));
    }
    let keys = client_handshake(stream, credentials)?;
    let digest: [u8; 32] = Sha256::digest(module).into();
    let mut begin = Vec::with_capacity(40);
    begin.extend_from_slice(&(module.len() as u64).to_le_bytes());
    begin.extend_from_slice(&digest);
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_BEGIN,
        request_id,
        0,
        &begin,
    )?;
    let chunk_limit = limits.chunk_bytes_limit().max(1);
    let mut sequence = 1_u64;
    for (chunk_index, bytes) in module.chunks(chunk_limit).enumerate() {
        let offset = chunk_index
            .checked_mul(chunk_limit)
            .ok_or(ProtocolError::InvalidFrame("module chunk offset overflow"))?;
        let mut chunk = Vec::with_capacity(8 + bytes.len());
        chunk.extend_from_slice(&(offset as u64).to_le_bytes());
        chunk.extend_from_slice(bytes);
        write_frame(
            stream,
            &keys.client_to_server,
            credentials.session_id(),
            KIND_MODULE_CHUNK,
            request_id,
            sequence,
            &chunk,
        )?;
        sequence = sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidFrame("frame sequence overflow"))?;
    }
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_END,
        request_id,
        sequence,
        &[],
    )?;
    stream.flush()?;

    let acknowledgement = read_frame(stream, &keys.server_to_client, credentials.session_id(), 40)?;
    if acknowledgement.kind != KIND_UPLOAD_ACCEPTED
        || acknowledgement.request_id != request_id
        || acknowledgement.sequence != 0
        || acknowledgement.payload.len() != 40
    {
        return Err(ProtocolError::InvalidFrame(
            "invalid upload acknowledgement",
        ));
    }
    let acknowledged_len = usize_from_u64(read_u64(&acknowledgement.payload[0..8])?)?;
    let mut acknowledged_digest = [0_u8; 32];
    acknowledged_digest.copy_from_slice(&acknowledgement.payload[8..40]);
    if acknowledged_len != module.len() || !bool::from(acknowledged_digest.ct_eq(&digest)) {
        return Err(ProtocolError::InvalidFrame(
            "acknowledgement does not match upload",
        ));
    }

    Ok(TransferAcknowledgement {
        request_id,
        module_len: acknowledged_len,
        module_digest: acknowledged_digest,
    })
}

/// Sends one complete reload command and waits for its safe-point result.
///
/// Upload acknowledgement is deliberately not returned as a terminal status.
/// This call succeeds only after the app reports committed, rejected, or
/// cancelled through the authenticated response direction.
pub fn send_reload_command<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    request_id: u64,
    metadata: ModuleMetadata,
    module: &[u8],
) -> Result<ReloadResult, ProtocolError>
where
    S: Read + Write,
{
    validate_outgoing_module(limits, module)?;
    let keys = client_handshake(stream, credentials)?;
    let digest: [u8; 32] = Sha256::digest(module).into();
    let mut begin = Vec::with_capacity(RELOAD_BEGIN_LEN);
    begin.extend_from_slice(&(module.len() as u64).to_le_bytes());
    begin.extend_from_slice(&digest);
    encode_metadata(metadata, &mut begin);
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_RELOAD_BEGIN,
        request_id,
        0,
        &begin,
    )?;
    let end_sequence = write_module_chunks(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        limits,
        request_id,
        module,
    )?;
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_MODULE_END,
        request_id,
        end_sequence,
        &[],
    )?;
    stream.flush()?;

    let acknowledgement = read_frame(stream, &keys.server_to_client, credentials.session_id(), 40)?;
    validate_acknowledgement(&acknowledgement, request_id, module.len(), &digest)?;
    let max_result_bytes = limits
        .diagnostic_bytes_limit()
        .checked_add(20)
        .ok_or(ProtocolError::InvalidFrame("reload result limit overflow"))?;
    let terminal = read_frame(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        max_result_bytes,
    )?;
    if terminal.kind != KIND_RELOAD_RESULT
        || terminal.request_id != request_id
        || terminal.sequence != 1
    {
        return Err(ProtocolError::InvalidFrame("invalid reload-result frame"));
    }
    ReloadResult::decode(&terminal.payload, limits.diagnostic_bytes_limit())
}

/// Receives one authenticated command and returns its terminal host result.
///
/// The command callback is entered only after all chunks, total length, and
/// digest validate. The accepted-upload frame is sent first; the callback's
/// result is then reported as the authoritative terminal outcome.
pub fn receive_reload_command<S, F>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    execute: F,
) -> Result<ReloadResult, ProtocolError>
where
    S: Read + Write,
    F: FnOnce(ReloadCommand) -> ReloadResult,
{
    let keys = server_handshake(stream, credentials)?;
    let begin = read_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        RELOAD_BEGIN_LEN,
    )?;
    receive_reload_command_after_begin(stream, credentials, limits, &keys, begin, execute)
}

/// Serves either a complete command or a reconnect result query.
pub fn receive_reload_connection<S, F, Q>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    execute: F,
    query: Q,
) -> Result<ReloadConnectionOutcome, ProtocolError>
where
    S: Read + Write,
    F: FnOnce(ReloadCommand) -> ReloadResult,
    Q: FnOnce(u64) -> Option<ReloadResult>,
{
    let keys = server_handshake(stream, credentials)?;
    let first = read_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        RELOAD_BEGIN_LEN,
    )?;
    match first.kind {
        KIND_RELOAD_BEGIN => Ok(ReloadConnectionOutcome::Command(
            receive_reload_command_after_begin(stream, credentials, limits, &keys, first, execute)?,
        )),
        KIND_RESULT_QUERY => {
            if first.sequence != 0 || !first.payload.is_empty() {
                return Err(ProtocolError::InvalidFrame("invalid result-query frame"));
            }
            let result = query(first.request_id);
            match &result {
                Some(result) => {
                    let payload = result.encode(limits.diagnostic_bytes_limit())?;
                    write_frame(
                        stream,
                        &keys.server_to_client,
                        credentials.session_id(),
                        KIND_RELOAD_RESULT,
                        first.request_id,
                        0,
                        &payload,
                    )?;
                }
                None => write_frame(
                    stream,
                    &keys.server_to_client,
                    credentials.session_id(),
                    KIND_RESULT_UNAVAILABLE,
                    first.request_id,
                    0,
                    &[],
                )?,
            }
            stream.flush()?;
            Ok(ReloadConnectionOutcome::Query(result))
        }
        _ => Err(ProtocolError::InvalidFrame(
            "unexpected authenticated command kind",
        )),
    }
}

/// Queries a terminal result after reconnecting to the same app session.
pub fn query_reload_result<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    request_id: u64,
) -> Result<Option<ReloadResult>, ProtocolError>
where
    S: Read + Write,
{
    let keys = client_handshake(stream, credentials)?;
    write_frame(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        KIND_RESULT_QUERY,
        request_id,
        0,
        &[],
    )?;
    stream.flush()?;
    let max_result_bytes = limits
        .diagnostic_bytes_limit()
        .checked_add(20)
        .ok_or(ProtocolError::InvalidFrame("reload result limit overflow"))?;
    let response = read_frame(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        max_result_bytes,
    )?;
    if response.request_id != request_id || response.sequence != 0 {
        return Err(ProtocolError::InvalidFrame("invalid result-query response"));
    }
    match response.kind {
        KIND_RELOAD_RESULT => Ok(Some(ReloadResult::decode(
            &response.payload,
            limits.diagnostic_bytes_limit(),
        )?)),
        KIND_RESULT_UNAVAILABLE if response.payload.is_empty() => Ok(None),
        _ => Err(ProtocolError::InvalidFrame("invalid result-query response")),
    }
}

fn receive_reload_command_after_begin<S, F>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    keys: &DirectionalKeys,
    begin: Frame,
    execute: F,
) -> Result<ReloadResult, ProtocolError>
where
    S: Read + Write,
    F: FnOnce(ReloadCommand) -> ReloadResult,
{
    if begin.kind != KIND_RELOAD_BEGIN
        || begin.sequence != 0
        || begin.payload.len() != RELOAD_BEGIN_LEN
    {
        return Err(ProtocolError::InvalidFrame("invalid reload-begin frame"));
    }
    let module_len = usize_from_u64(read_u64(&begin.payload[0..8])?)?;
    if module_len > limits.max_module_bytes() {
        return Err(ProtocolError::ModuleTooLarge {
            actual: module_len,
            maximum: limits.max_module_bytes(),
        });
    }
    let mut declared_digest = [0_u8; 32];
    declared_digest.copy_from_slice(&begin.payload[8..40]);
    let metadata = decode_metadata(&begin.payload[40..RELOAD_BEGIN_LEN])?;
    let (module, actual_digest, end_sequence) = read_module_chunks(
        stream,
        &keys.client_to_server,
        credentials.session_id(),
        limits,
        begin.request_id,
        module_len,
    )?;
    let end = read_frame(stream, &keys.client_to_server, credentials.session_id(), 0)?;
    if end.kind != KIND_MODULE_END
        || end.request_id != begin.request_id
        || end.sequence != end_sequence
        || !end.payload.is_empty()
    {
        return Err(ProtocolError::InvalidFrame("invalid module-end frame"));
    }
    if !bool::from(actual_digest.ct_eq(&declared_digest)) {
        return Err(ProtocolError::DigestMismatch);
    }

    write_acknowledgement(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        begin.request_id,
        module_len,
        &actual_digest,
    )?;
    stream.flush()?;
    let result = execute(ReloadCommand::from_parts(
        begin.request_id,
        metadata,
        actual_digest,
        module,
    ));
    let payload = result.encode(limits.diagnostic_bytes_limit())?;
    write_frame(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        KIND_RELOAD_RESULT,
        begin.request_id,
        1,
        &payload,
    )?;
    stream.flush()?;
    Ok(result)
}

/// Authenticates, receives, dispatches, and acknowledges one complete module.
pub fn receive_module_and_acknowledge<S, F>(
    stream: &mut S,
    credentials: &SessionCredentials,
    limits: ProtocolLimits,
    accept: F,
) -> Result<TransferAcknowledgement, ProtocolError>
where
    S: Read + Write,
    F: FnOnce(Vec<u8>) -> Result<(), String>,
{
    let keys = server_handshake(stream, credentials)?;
    let begin = read_frame(stream, &keys.client_to_server, credentials.session_id(), 40)?;
    if begin.kind != KIND_MODULE_BEGIN || begin.sequence != 0 || begin.payload.len() != 40 {
        return Err(ProtocolError::InvalidFrame("invalid module-begin frame"));
    }
    let module_len = usize_from_u64(read_u64(&begin.payload[0..8])?)?;
    if module_len > limits.max_module_bytes() {
        return Err(ProtocolError::ModuleTooLarge {
            actual: module_len,
            maximum: limits.max_module_bytes(),
        });
    }
    let mut declared_digest = [0_u8; 32];
    declared_digest.copy_from_slice(&begin.payload[8..40]);
    if module_len != 0 && limits.chunk_bytes_limit() == 0 {
        return Err(ProtocolError::InvalidFrame(
            "non-empty module requires a nonzero chunk limit",
        ));
    }

    let mut module = Vec::with_capacity(module_len);
    let mut digest = Sha256::new();
    let mut sequence = 1_u64;
    while module.len() < module_len {
        let remaining = module_len - module.len();
        let chunk_bytes = remaining.min(limits.chunk_bytes_limit());
        let chunk_payload_len = chunk_bytes
            .checked_add(8)
            .ok_or(ProtocolError::InvalidFrame("module chunk length overflow"))?;
        let chunk = read_frame(
            stream,
            &keys.client_to_server,
            credentials.session_id(),
            chunk_payload_len,
        )?;
        if chunk.kind != KIND_MODULE_CHUNK
            || chunk.request_id != begin.request_id
            || chunk.sequence != sequence
            || chunk.payload.len() < 8
            || chunk.payload.len() > chunk_payload_len
            || usize_from_u64(read_u64(&chunk.payload[0..8])?)? != module.len()
            || chunk.payload.len() == 8
        {
            return Err(ProtocolError::InvalidFrame("invalid module-chunk frame"));
        }
        let bytes = &chunk.payload[8..];
        digest.update(bytes);
        module.extend_from_slice(bytes);
        sequence = sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidFrame("frame sequence overflow"))?;
    }

    let end = read_frame(stream, &keys.client_to_server, credentials.session_id(), 0)?;
    if end.kind != KIND_MODULE_END
        || end.request_id != begin.request_id
        || end.sequence != sequence
        || !end.payload.is_empty()
    {
        return Err(ProtocolError::InvalidFrame("invalid module-end frame"));
    }
    let actual_digest: [u8; 32] = digest.finalize().into();
    if !bool::from(actual_digest.ct_eq(&declared_digest)) {
        return Err(ProtocolError::DigestMismatch);
    }

    accept(module).map_err(ProtocolError::SinkRejected)?;
    let mut acknowledgement_payload = Vec::with_capacity(40);
    acknowledgement_payload.extend_from_slice(&(module_len as u64).to_le_bytes());
    acknowledgement_payload.extend_from_slice(&actual_digest);
    write_frame(
        stream,
        &keys.server_to_client,
        credentials.session_id(),
        KIND_UPLOAD_ACCEPTED,
        begin.request_id,
        0,
        &acknowledgement_payload,
    )?;
    stream.flush()?;

    Ok(TransferAcknowledgement {
        request_id: begin.request_id,
        module_len,
        module_digest: actual_digest,
    })
}

fn validate_outgoing_module(limits: ProtocolLimits, module: &[u8]) -> Result<(), ProtocolError> {
    if module.len() > limits.max_module_bytes() {
        return Err(ProtocolError::ModuleTooLarge {
            actual: module.len(),
            maximum: limits.max_module_bytes(),
        });
    }
    if !module.is_empty() && limits.chunk_bytes_limit() == 0 {
        return Err(ProtocolError::InvalidFrame(
            "non-empty module requires a nonzero chunk limit",
        ));
    }
    Ok(())
}

fn write_module_chunks<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    limits: ProtocolLimits,
    request_id: u64,
    module: &[u8],
) -> Result<u64, ProtocolError>
where
    S: Write,
{
    let chunk_limit = limits.chunk_bytes_limit().max(1);
    let mut sequence = 1_u64;
    for (chunk_index, bytes) in module.chunks(chunk_limit).enumerate() {
        let offset = chunk_index
            .checked_mul(chunk_limit)
            .ok_or(ProtocolError::InvalidFrame("module chunk offset overflow"))?;
        let mut chunk = Vec::with_capacity(8 + bytes.len());
        chunk.extend_from_slice(&(offset as u64).to_le_bytes());
        chunk.extend_from_slice(bytes);
        write_frame(
            stream,
            keys,
            session_id,
            KIND_MODULE_CHUNK,
            request_id,
            sequence,
            &chunk,
        )?;
        sequence = sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidFrame("frame sequence overflow"))?;
    }
    Ok(sequence)
}

fn read_module_chunks<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    limits: ProtocolLimits,
    request_id: u64,
    module_len: usize,
) -> Result<(Vec<u8>, [u8; 32], u64), ProtocolError>
where
    S: Read,
{
    if module_len != 0 && limits.chunk_bytes_limit() == 0 {
        return Err(ProtocolError::InvalidFrame(
            "non-empty module requires a nonzero chunk limit",
        ));
    }
    let mut module = Vec::with_capacity(module_len);
    let mut digest = Sha256::new();
    let mut sequence = 1_u64;
    while module.len() < module_len {
        let remaining = module_len - module.len();
        let chunk_payload_limit = remaining
            .min(limits.chunk_bytes_limit())
            .checked_add(8)
            .ok_or(ProtocolError::InvalidFrame("module chunk length overflow"))?;
        let chunk = read_frame(stream, keys, session_id, chunk_payload_limit)?;
        if chunk.kind != KIND_MODULE_CHUNK
            || chunk.request_id != request_id
            || chunk.sequence != sequence
            || chunk.payload.len() < 9
            || chunk.payload.len() > chunk_payload_limit
            || usize_from_u64(read_u64(&chunk.payload[0..8])?)? != module.len()
        {
            return Err(ProtocolError::InvalidFrame("invalid module-chunk frame"));
        }
        let bytes = &chunk.payload[8..];
        digest.update(bytes);
        module.extend_from_slice(bytes);
        sequence = sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidFrame("frame sequence overflow"))?;
    }
    Ok((module, digest.finalize().into(), sequence))
}

fn validate_acknowledgement(
    frame: &Frame,
    request_id: u64,
    module_len: usize,
    module_digest: &[u8; 32],
) -> Result<(), ProtocolError> {
    if frame.kind != KIND_UPLOAD_ACCEPTED
        || frame.request_id != request_id
        || frame.sequence != 0
        || frame.payload.len() != 40
        || usize_from_u64(read_u64(&frame.payload[0..8])?)? != module_len
        || !bool::from(frame.payload[8..40].ct_eq(module_digest))
    {
        return Err(ProtocolError::InvalidFrame(
            "invalid upload acknowledgement",
        ));
    }
    Ok(())
}

fn write_acknowledgement<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    request_id: u64,
    module_len: usize,
    module_digest: &[u8; 32],
) -> Result<(), ProtocolError>
where
    S: Write,
{
    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(&(module_len as u64).to_le_bytes());
    payload.extend_from_slice(module_digest);
    write_frame(
        stream,
        keys,
        session_id,
        KIND_UPLOAD_ACCEPTED,
        request_id,
        0,
        &payload,
    )
}

fn encode_metadata(metadata: ModuleMetadata, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&metadata.application_id());
    bytes.extend_from_slice(&metadata.build_id());
    let (abi_major, abi_minor) = metadata.abi_version();
    bytes.extend_from_slice(&abi_major.to_le_bytes());
    bytes.extend_from_slice(&abi_minor.to_le_bytes());
    bytes.extend_from_slice(&metadata.capability_manifest_digest());
}

fn decode_metadata(bytes: &[u8]) -> Result<ModuleMetadata, ProtocolError> {
    if bytes.len() != 68 {
        return Err(ProtocolError::InvalidFrame(
            "invalid module metadata length",
        ));
    }
    let mut application_id = [0_u8; 16];
    application_id.copy_from_slice(&bytes[0..16]);
    let mut build_id = [0_u8; 16];
    build_id.copy_from_slice(&bytes[16..32]);
    let abi_major = read_u16(&bytes[32..34])?;
    let abi_minor = read_u16(&bytes[34..36])?;
    let mut capability_manifest_digest = [0_u8; 32];
    capability_manifest_digest.copy_from_slice(&bytes[36..68]);
    Ok(ModuleMetadata::new(
        application_id,
        build_id,
        abi_major,
        abi_minor,
        capability_manifest_digest,
    ))
}

fn random_array() -> Result<[u8; 32], ProtocolError> {
    let mut bytes = [0_u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| ProtocolError::Cryptography)?;
    Ok(bytes)
}

fn read_u16(bytes: &[u8]) -> Result<u16, ProtocolError> {
    let bytes: [u8; 2] = bytes
        .try_into()
        .map_err(|_| ProtocolError::InvalidFrame("truncated u16"))?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u64(bytes: &[u8]) -> Result<u64, ProtocolError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| ProtocolError::InvalidFrame("truncated u64"))?;
    Ok(u64::from_le_bytes(bytes))
}

fn usize_from_u64(value: u64) -> Result<usize, ProtocolError> {
    value
        .try_into()
        .map_err(|_| ProtocolError::InvalidFrame("length does not fit this target"))
}

#[cfg(test)]
mod tests;
