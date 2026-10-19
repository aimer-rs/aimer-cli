use super::*;

pub(super) fn client_handshake<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
) -> Result<DirectionalKeys, ProtocolError>
where
    S: Read + Write,
{
    let mut challenge = [0_u8; CHALLENGE_LEN];
    stream.read_exact(&mut challenge)?;
    if &challenge[0..4] != HANDSHAKE_MAGIC
        || read_u16(&challenge[4..6])? != PROTOCOL_MAJOR
        || read_u16(&challenge[6..8])? != PROTOCOL_MINOR
    {
        return Err(ProtocolError::Authentication);
    }
    let mut server_nonce = [0_u8; 32];
    server_nonce.copy_from_slice(&challenge[8..40]);
    let client_nonce = random_array()?;
    let transcript = transcript(credentials.session_id(), &server_nonce, &client_nonce);
    let client_tag = authentication_tag(credentials, b"client", &transcript);
    let mut client_auth = [0_u8; CLIENT_AUTH_LEN];
    client_auth[0..4].copy_from_slice(HANDSHAKE_MAGIC);
    client_auth[4..20].copy_from_slice(credentials.session_id());
    client_auth[20..52].copy_from_slice(&client_nonce);
    client_auth[52..54].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    client_auth[54..56].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    client_auth[56..88].copy_from_slice(client_tag.as_ref());
    stream.write_all(&client_auth)?;
    stream.flush()?;

    let mut server_auth = [0_u8; SERVER_AUTH_LEN];
    stream.read_exact(&mut server_auth)?;
    if &server_auth[0..4] != HANDSHAKE_MAGIC
        || read_u16(&server_auth[4..6])? != PROTOCOL_MAJOR
        || read_u16(&server_auth[6..8])? != PROTOCOL_MINOR
    {
        return Err(ProtocolError::Authentication);
    }
    verify_authentication_tag(credentials, b"server", &transcript, &server_auth[8..40])?;
    derive_keys(credentials, &transcript)
}

pub(super) fn server_handshake<S>(
    stream: &mut S,
    credentials: &SessionCredentials,
) -> Result<DirectionalKeys, ProtocolError>
where
    S: Read + Write,
{
    let server_nonce = random_array()?;
    let mut challenge = [0_u8; CHALLENGE_LEN];
    challenge[0..4].copy_from_slice(HANDSHAKE_MAGIC);
    challenge[4..6].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    challenge[6..8].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    challenge[8..40].copy_from_slice(&server_nonce);
    stream.write_all(&challenge)?;
    stream.flush()?;

    let mut client_auth = [0_u8; CLIENT_AUTH_LEN];
    stream.read_exact(&mut client_auth)?;
    if &client_auth[0..4] != HANDSHAKE_MAGIC
        || !bool::from(client_auth[4..20].ct_eq(credentials.session_id()))
        || read_u16(&client_auth[52..54])? != PROTOCOL_MAJOR
        || read_u16(&client_auth[54..56])? != PROTOCOL_MINOR
    {
        return Err(ProtocolError::Authentication);
    }
    let mut client_nonce = [0_u8; 32];
    client_nonce.copy_from_slice(&client_auth[20..52]);
    let transcript = transcript(credentials.session_id(), &server_nonce, &client_nonce);
    verify_authentication_tag(credentials, b"client", &transcript, &client_auth[56..88])?;
    let server_tag = authentication_tag(credentials, b"server", &transcript);
    let mut server_auth = [0_u8; SERVER_AUTH_LEN];
    server_auth[0..4].copy_from_slice(HANDSHAKE_MAGIC);
    server_auth[4..6].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    server_auth[6..8].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    server_auth[8..40].copy_from_slice(server_tag.as_ref());
    stream.write_all(&server_auth)?;
    stream.flush()?;
    derive_keys(credentials, &transcript)
}

pub(super) fn transcript(
    session_id: &[u8; 16],
    server_nonce: &[u8; 32],
    client_nonce: &[u8; 32],
) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(84);
    transcript.extend_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    transcript.extend_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    transcript.extend_from_slice(session_id);
    transcript.extend_from_slice(server_nonce);
    transcript.extend_from_slice(client_nonce);
    transcript
}

pub(super) fn authentication_tag(
    credentials: &SessionCredentials,
    role: &[u8],
    transcript: &[u8],
) -> hmac::Tag {
    let key = hmac::Key::new(hmac::HMAC_SHA256, credentials.token.as_ref());
    let mut authenticated = Vec::with_capacity(role.len() + transcript.len());
    authenticated.extend_from_slice(role);
    authenticated.extend_from_slice(transcript);
    hmac::sign(&key, &authenticated)
}

pub(super) fn verify_authentication_tag(
    credentials: &SessionCredentials,
    role: &[u8],
    transcript: &[u8],
    tag: &[u8],
) -> Result<(), ProtocolError> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, credentials.token.as_ref());
    let mut authenticated = Vec::with_capacity(role.len() + transcript.len());
    authenticated.extend_from_slice(role);
    authenticated.extend_from_slice(transcript);
    hmac::verify(&key, &authenticated, tag).map_err(|_| ProtocolError::Authentication)
}

pub(super) fn derive_keys(
    credentials: &SessionCredentials,
    transcript: &[u8],
) -> Result<DirectionalKeys, ProtocolError> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, transcript);
    let pseudo_random_key = salt.extract(credentials.token.as_ref());
    let client_to_server = derive_frame_keys(
        &pseudo_random_key,
        b"aimer-reload-client-to-server-auth",
        b"aimer-reload-client-to-server-encryption",
    )?;
    let server_to_client = derive_frame_keys(
        &pseudo_random_key,
        b"aimer-reload-server-to-client-auth",
        b"aimer-reload-server-to-client-encryption",
    )?;
    Ok(DirectionalKeys {
        client_to_server,
        server_to_client,
    })
}

pub(super) fn derive_frame_keys(
    pseudo_random_key: &hkdf::Prk,
    authentication_label: &'static [u8],
    encryption_label: &'static [u8],
) -> Result<FrameKeys, ProtocolError> {
    let mut authentication_bytes = expand_key(pseudo_random_key, authentication_label)?;
    let authentication = hmac::Key::new(hmac::HMAC_SHA256, &authentication_bytes);
    authentication_bytes.zeroize();
    let mut encryption_bytes = expand_key(pseudo_random_key, encryption_label)?;
    let unbound = aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &encryption_bytes)
        .map_err(|_| ProtocolError::Cryptography)?;
    encryption_bytes.zeroize();
    Ok(FrameKeys {
        authentication,
        encryption: aead::LessSafeKey::new(unbound),
    })
}

pub(super) fn expand_key(
    pseudo_random_key: &hkdf::Prk,
    label: &'static [u8],
) -> Result<[u8; 32], ProtocolError> {
    let labels = [label];
    let output = pseudo_random_key
        .expand(&labels, FrameKeyLength)
        .map_err(|_| ProtocolError::Cryptography)?;
    let mut key = [0_u8; 32];
    output
        .fill(&mut key)
        .map_err(|_| ProtocolError::Cryptography)?;
    Ok(key)
}

pub(super) fn write_frame<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    kind: u16,
    request_id: u64,
    sequence: u64,
    payload: &[u8],
) -> Result<(), ProtocolError>
where
    S: Write,
{
    let encrypted_len = payload
        .len()
        .checked_add(aead::CHACHA20_POLY1305.tag_len())
        .ok_or(ProtocolError::InvalidFrame(
            "encrypted payload length overflow",
        ))?;
    let mut header = [0_u8; FRAME_HEADER_LEN];
    header[0..4].copy_from_slice(FRAME_MAGIC);
    header[4..6].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    header[6..8].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());
    header[8..10].copy_from_slice(&kind.to_le_bytes());
    header[12..14].copy_from_slice(&(FRAME_HEADER_LEN as u16).to_le_bytes());
    header[16..24].copy_from_slice(&(encrypted_len as u64).to_le_bytes());
    header[24..40].copy_from_slice(session_id);
    header[40..48].copy_from_slice(&request_id.to_le_bytes());
    header[48..56].copy_from_slice(&sequence.to_le_bytes());
    let mut encrypted_payload = Vec::with_capacity(encrypted_len);
    encrypted_payload.extend_from_slice(payload);
    keys.encryption
        .seal_in_place_append_tag(
            frame_nonce(sequence),
            aead::Aad::from(&header),
            &mut encrypted_payload,
        )
        .map_err(|_| ProtocolError::Cryptography)?;
    let mut authenticated = Vec::with_capacity(FRAME_HEADER_LEN + encrypted_payload.len());
    authenticated.extend_from_slice(&header);
    authenticated.extend_from_slice(&encrypted_payload);
    let tag = hmac::sign(&keys.authentication, &authenticated);
    header[AUTH_TAG_START..AUTH_TAG_END].copy_from_slice(tag.as_ref());
    stream.write_all(&header)?;
    stream.write_all(&encrypted_payload)?;
    Ok(())
}

pub(super) fn read_frame<S>(
    stream: &mut S,
    keys: &FrameKeys,
    session_id: &[u8; 16],
    max_payload: usize,
) -> Result<Frame, ProtocolError>
where
    S: Read,
{
    let mut header = [0_u8; FRAME_HEADER_LEN];
    stream.read_exact(&mut header)?;
    if &header[0..4] != FRAME_MAGIC
        || read_u16(&header[4..6])? != PROTOCOL_MAJOR
        || read_u16(&header[6..8])? != PROTOCOL_MINOR
        || read_u16(&header[10..12])? != 0
        || read_u16(&header[12..14])? as usize != FRAME_HEADER_LEN
        || read_u16(&header[14..16])? != 0
        || !bool::from(header[24..40].ct_eq(session_id))
    {
        return Err(ProtocolError::InvalidFrame("invalid authenticated header"));
    }
    let payload_len = usize_from_u64(read_u64(&header[16..24])?)?;
    let max_encrypted_payload = max_payload
        .checked_add(aead::CHACHA20_POLY1305.tag_len())
        .ok_or(ProtocolError::InvalidFrame("payload limit overflow"))?;
    if payload_len < aead::CHACHA20_POLY1305.tag_len() || payload_len > max_encrypted_payload {
        return Err(ProtocolError::InvalidFrame("payload exceeds message limit"));
    }
    let mut payload = vec![0_u8; payload_len];
    stream.read_exact(&mut payload)?;
    let mut tag = [0_u8; 32];
    tag.copy_from_slice(&header[AUTH_TAG_START..AUTH_TAG_END]);
    header[AUTH_TAG_START..AUTH_TAG_END].fill(0);
    let mut authenticated = Vec::with_capacity(FRAME_HEADER_LEN + payload_len);
    authenticated.extend_from_slice(&header);
    authenticated.extend_from_slice(&payload);
    hmac::verify(&keys.authentication, &authenticated, &tag)
        .map_err(|_| ProtocolError::Authentication)?;

    let sequence = read_u64(&header[48..56])?;
    let plaintext = keys
        .encryption
        .open_in_place(
            frame_nonce(sequence),
            aead::Aad::from(&header),
            &mut payload,
        )
        .map_err(|_| ProtocolError::Authentication)?;
    let plaintext_len = plaintext.len();
    payload.truncate(plaintext_len);

    Ok(Frame {
        kind: read_u16(&header[8..10])?,
        request_id: read_u64(&header[40..48])?,
        sequence,
        payload,
    })
}

pub(super) fn frame_nonce(sequence: u64) -> aead::Nonce {
    let mut nonce = [0_u8; 12];
    nonce[4..12].copy_from_slice(&sequence.to_be_bytes());
    aead::Nonce::assume_unique_for_key(nonce)
}
