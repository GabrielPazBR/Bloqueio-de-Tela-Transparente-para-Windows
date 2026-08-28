use bloqueio_transparente::protocol::{
    ClientRequest, CommandCodec, MAX_FRAME_BYTES, PipeNames, ProtocolError, ServiceResponse,
};

#[test]
fn public_protocol_has_no_unlock_command() {
    let requests = [
        ClientRequest::Lock,
        ClientRequest::Status,
        ClientRequest::Heartbeat { locked: true },
        ClientRequest::VerifyPassword {
            candidate: "segredo".into(),
        },
    ];

    for request in requests {
        let encoded = CommandCodec::encode_request(&request).unwrap();
        assert!(
            !String::from_utf8_lossy(&encoded)
                .to_lowercase()
                .contains("unlock")
        );
    }
}

#[test]
fn codec_round_trips_a_bounded_length_prefixed_frame() {
    let request = ClientRequest::Heartbeat { locked: true };
    let encoded = CommandCodec::encode_request(&request).unwrap();

    assert!(encoded.len() <= MAX_FRAME_BYTES + 4);
    assert_eq!(CommandCodec::decode_request(&encoded).unwrap(), request);

    let response = ServiceResponse::Status {
        enabled: true,
        agent_running: true,
        locked: true,
        last_error: None,
    };
    let encoded = CommandCodec::encode_response(&response).unwrap();
    assert_eq!(CommandCodec::decode_response(&encoded).unwrap(), response);
}

#[test]
fn oversized_frames_are_rejected_before_json_parsing() {
    let declared = (MAX_FRAME_BYTES as u32 + 1).to_le_bytes();
    assert_eq!(
        CommandCodec::decode_request(&declared).unwrap_err(),
        ProtocolError::FrameTooLarge
    );
}

#[test]
fn pipe_names_are_local_and_unique_per_session() {
    let names = PipeNames::for_session(7);
    assert_eq!(names.control, r"\\.\pipe\BloqueioTransparente.Control.7");
    assert_eq!(names.agent, r"\\.\pipe\BloqueioTransparente.Agent.7");
}
