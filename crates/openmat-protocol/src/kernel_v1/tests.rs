use serde_json::{Value, json};

use super::*;
use crate::{
    Event, ExecutionMode, ImplementationInfo, InterruptRequest, MAX_PREVIEW_ELEMENTS, MessageKind,
    PreviewTruncation, ProtocolError, StatusEvent,
};

fn test_client() -> ImplementationInfo {
    ImplementationInfo {
        name: "openmat-web".to_owned(),
        version: "1.0.0".to_owned(),
    }
}

fn test_kernel() -> ImplementationInfo {
    ImplementationInfo {
        name: "openmat-kernel".to_owned(),
        version: "1.0.0".to_owned(),
    }
}

fn v1_capabilities() -> Capabilities {
    Capabilities {
        execution_modes: vec![
            ExecutionMode::File,
            ExecutionMode::Cell,
            ExecutionMode::Repl,
        ],
        display_mime_types: vec!["text/plain".to_owned()],
        max_preview_elements: MAX_PREVIEW_ELEMENTS,
        max_string_element_code_units: Some(MAX_STRING_ELEMENT_CODE_UNITS),
        max_preview_code_units: Some(MAX_PREVIEW_CODE_UNITS),
        interrupt: true,
        workspace_delta: true,
    }
}

fn bootstrap_request() -> BootstrapRequestEnvelope {
    BootstrapRequestEnvelope::new(
        "session-1",
        "client-1",
        InitializeRequest::v1(test_client(), v1_capabilities()),
    )
}

fn inspect_request(max_elements: u64) -> RequestEnvelope {
    RequestEnvelope::new(
        "session-1",
        "request-1",
        Request::Inspect(InspectRequest {
            name: "answer".to_owned(),
            range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            max_elements,
        }),
    )
}

fn preview(
    class: &str,
    complex: bool,
    values: Vec<PreviewValue>,
    selected_count: u64,
    omitted_elements: u64,
) -> MatrixPreview {
    MatrixPreview {
        class: class.to_owned(),
        dimensions: vec![1, selected_count],
        complex,
        selected_range: MatrixRange {
            start: vec![1, 1],
            size: vec![1, selected_count],
        },
        values,
        truncation: PreviewTruncation {
            truncated: omitted_elements != 0,
            omitted_elements,
        },
    }
}

fn response_with_preview(preview: MatrixPreview) -> ResponseEnvelope {
    ResponseEnvelope::success(
        &inspect_request(MAX_PREVIEW_ELEMENTS),
        "response-1",
        ResponseResult::Inspect(preview),
    )
}

fn test_len(value: u64) -> usize {
    usize::try_from(value).expect("protocol hard limits fit usize on test targets")
}

#[test]
fn golden_bootstrap_request_and_response_stay_in_v0_envelopes() {
    let request = bootstrap_request();
    let request_json = encode_bootstrap_request(&request).expect("encode bootstrap request");
    let request_value: Value = serde_json::from_str(&request_json).expect("request JSON");
    assert_eq!(
        request_value,
        json!({
            "protocol": "openmat-kernel-v0",
            "sessionId": "session-1",
            "messageId": "client-1",
            "kind": "request",
            "request": {
                "type": "initialize",
                "params": {
                    "client": {"name": "openmat-web", "version": "1.0.0"},
                    "supportedProtocols": ["openmat-kernel-v1", "openmat-kernel-v0"],
                    "capabilities": {
                        "executionModes": ["file", "cell", "repl"],
                        "displayMimeTypes": ["text/plain"],
                        "maxPreviewElements": 4096,
                        "maxStringElementCodeUnits": 16384,
                        "maxPreviewCodeUnits": 65536,
                        "interrupt": true,
                        "workspaceDelta": true
                    }
                }
            }
        })
    );
    assert_eq!(
        decode_bootstrap_request(&request_json).expect("decode bootstrap request"),
        request
    );

    let negotiated = negotiate_initialize(
        match &request.request {
            BootstrapRequest::Initialize(initialize) => initialize,
        },
        &v1_capabilities(),
    )
    .expect("negotiate v1");
    assert_eq!(negotiated.protocol, ProtocolVersion::V1);
    let response = BootstrapResponseEnvelope::success(
        &request,
        "kernel-1",
        initialize_result(negotiated, test_kernel()),
    );
    let response_json = encode_bootstrap_response(&response).expect("encode bootstrap response");
    let response_value: Value = serde_json::from_str(&response_json).expect("response JSON");
    assert_eq!(response_value["protocol"], PROTOCOL_V0);
    assert_eq!(
        response_value["result"]["data"]["negotiatedProtocol"],
        PROTOCOL_V1
    );
    assert_eq!(
        validate_initialize_exchange(&request, &response).expect("valid exchange"),
        ProtocolVersion::V1
    );

    let first_post_bootstrap = EventEnvelope::new(
        "session-1",
        "kernel-2",
        Event::Status(StatusEvent {
            status: crate::KernelStatus::Idle,
        }),
    );
    assert_eq!(first_post_bootstrap.protocol, PROTOCOL_V1);
    first_post_bootstrap
        .validate()
        .expect("valid v1 idle event");
}

#[test]
fn negotiation_is_ordered_deterministic_and_strips_v1_fields_on_v0() {
    assert_eq!(
        select_protocol(&[
            "openmat-kernel-v2".to_owned(),
            PROTOCOL_V0.to_owned(),
            PROTOCOL_V1.to_owned(),
        ])
        .expect("first common protocol"),
        ProtocolVersion::V0
    );
    let request = InitializeRequest {
        client: test_client(),
        supported_protocols: vec![PROTOCOL_V0.to_owned()],
        capabilities: v1_capabilities(),
    };
    let negotiated = negotiate_initialize(&request, &v1_capabilities()).expect("v0 negotiation");
    assert_eq!(negotiated.protocol, ProtocolVersion::V0);
    assert_eq!(negotiated.capabilities.max_string_element_code_units, None);
    assert_eq!(negotiated.capabilities.max_preview_code_units, None);

    let envelope = BootstrapRequestEnvelope::new("s", "q", request);
    let response = BootstrapResponseEnvelope::success(
        &envelope,
        "r",
        initialize_result(negotiated, test_kernel()),
    );
    let json = encode_bootstrap_response(&response).expect("encode v0 selection");
    assert!(!json.contains("maxStringElementCodeUnits"));
    assert!(!json.contains("maxPreviewCodeUnits"));
    assert_eq!(
        validate_initialize_exchange(&envelope, &response).expect("valid v0 exchange"),
        ProtocolVersion::V0
    );
}

#[test]
fn negotiation_rejects_invalid_v1_offer_duplicates_and_no_common_version() {
    let missing_v1_limits = InitializeRequest::v1(test_client(), Capabilities::default());
    let error = negotiate_initialize(&missing_v1_limits, &v1_capabilities())
        .expect_err("v1 limits are required");
    assert_eq!(error.category(), "protocol.invalidCapabilities");

    let mut duplicate = InitializeRequest::v1(test_client(), v1_capabilities());
    duplicate.supported_protocols.push(PROTOCOL_V1.to_owned());
    assert_eq!(
        negotiate_initialize(&duplicate, &v1_capabilities())
            .expect_err("duplicate protocols")
            .category(),
        "protocol.invalidOffer"
    );

    let unknown_only = InitializeRequest {
        client: test_client(),
        supported_protocols: vec!["openmat-kernel-v9".to_owned()],
        capabilities: Capabilities::default(),
    };
    assert_eq!(
        negotiate_initialize(&unknown_only, &v1_capabilities())
            .expect_err("no common protocol")
            .category(),
        "protocol.noCommonVersion"
    );
}

#[test]
fn capability_negotiation_uses_componentwise_minima_and_hard_bounds() {
    let client = Capabilities {
        execution_modes: vec![ExecutionMode::Cell, ExecutionMode::Repl],
        display_mime_types: vec!["text/plain".to_owned(), "image/png".to_owned()],
        max_preview_elements: 200,
        max_string_element_code_units: Some(10_000),
        max_preview_code_units: Some(40_000),
        interrupt: true,
        workspace_delta: true,
    };
    let kernel = Capabilities {
        execution_modes: vec![ExecutionMode::File, ExecutionMode::Cell],
        display_mime_types: vec!["image/png".to_owned(), "text/html".to_owned()],
        max_preview_elements: 100,
        max_string_element_code_units: Some(8_000),
        max_preview_code_units: Some(32_000),
        interrupt: false,
        workspace_delta: true,
    };
    assert_eq!(
        Capabilities::negotiate(&client, &kernel, ProtocolVersion::V1)
            .expect("valid capability intersection"),
        Capabilities {
            execution_modes: vec![ExecutionMode::Cell],
            display_mime_types: vec!["image/png".to_owned()],
            max_preview_elements: 100,
            max_string_element_code_units: Some(8_000),
            max_preview_code_units: Some(32_000),
            interrupt: false,
            workspace_delta: true,
        }
    );

    for invalid in [
        PreviewLimits::new(0, 1, 1),
        PreviewLimits::new(MAX_PREVIEW_ELEMENTS + 1, 1, 1),
        PreviewLimits::new(1, MAX_STRING_ELEMENT_CODE_UNITS + 1, MAX_PREVIEW_CODE_UNITS),
        PreviewLimits::new(1, 2, 1),
        PreviewLimits::new(1, 1, MAX_PREVIEW_CODE_UNITS + 1),
    ] {
        assert!(invalid.is_err());
    }
}

#[test]
fn char_and_string_golden_previews_preserve_utf16_truth() {
    let char_preview = preview(
        "char",
        false,
        vec![
            PreviewValue::CharCodeUnit { value: 65 },
            PreviewValue::CharCodeUnit { value: 55_357 },
            PreviewValue::CharCodeUnit { value: 56_898 },
        ],
        3,
        0,
    );
    char_preview
        .validate(&PreviewLimits::default())
        .expect("surrogate code units are exact char values");
    assert_eq!(
        serde_json::to_value(&char_preview).expect("serialize char preview"),
        json!({
            "class": "char",
            "dimensions": [1, 3],
            "complex": false,
            "selectedRange": {"start": [1, 1], "size": [1, 3]},
            "values": [
                {"kind": "charCodeUnit", "value": 65},
                {"kind": "charCodeUnit", "value": 55357},
                {"kind": "charCodeUnit", "value": 56898}
            ],
            "truncation": {"truncated": false, "omittedElements": 0}
        })
    );

    let string_preview = preview(
        "string",
        false,
        vec![
            PreviewValue::String {
                code_units: vec![55_357],
                missing: false,
            },
            PreviewValue::String {
                code_units: Vec::new(),
                missing: false,
            },
            PreviewValue::String {
                code_units: Vec::new(),
                missing: true,
            },
        ],
        3,
        0,
    );
    string_preview
        .validate(&PreviewLimits::default())
        .expect("isolated surrogate, empty, and missing are distinct");
    let encoded = serde_json::to_value(&string_preview).expect("serialize string preview");
    assert_eq!(encoded["values"][0]["codeUnits"], json!([55_357]));
    assert_eq!(encoded["values"][1]["missing"], false);
    assert_eq!(encoded["values"][2]["missing"], true);
}

#[test]
fn exact_integer_previews_cover_uint64_max_int64_min_and_complex_storage() {
    let unsigned = preview(
        "uint64",
        false,
        vec![PreviewValue::Integer {
            real: u64::MAX.to_string(),
            imaginary: "0".to_owned(),
        }],
        1,
        0,
    );
    unsigned
        .validate(&PreviewLimits::default())
        .expect("uint64 max remains an exact decimal string");

    let signed_complex = preview(
        "int64",
        true,
        vec![
            PreviewValue::Integer {
                real: i64::MIN.to_string(),
                imaginary: "0".to_owned(),
            },
            PreviewValue::Integer {
                real: "7".to_owned(),
                imaginary: "-9".to_owned(),
            },
        ],
        2,
        0,
    );
    signed_complex
        .validate(&PreviewLimits::default())
        .expect("int64 minimum and complex integer are exact");
    let response = response_with_preview(signed_complex);
    let json = encode_response(&response, &PreviewLimits::default()).expect("encode response");
    let decoded = decode_response(&json, &PreviewLimits::default()).expect("decode response");
    assert_eq!(decoded, response);
    assert!(json.contains("-9223372036854775808"));
}

#[test]
fn integer_validation_rejects_noncanonical_signedness_and_range_errors() {
    let cases = [
        ("int64", "+1", "0"),
        ("int64", "01", "0"),
        ("int64", "-0", "0"),
        ("int64", "1.0", "0"),
        ("int64", "1e3", "0"),
        ("int64", " 1", "0"),
        ("int64", "", "0"),
        ("uint8", "-1", "0"),
        ("int8", "128", "0"),
        ("int8", "-129", "0"),
        ("uint8", "256", "0"),
        ("uint64", "18446744073709551616", "0"),
        ("int64", "0", "1"),
    ];
    for (class, real, imaginary) in cases {
        let invalid = preview(
            class,
            false,
            vec![PreviewValue::Integer {
                real: real.to_owned(),
                imaginary: imaginary.to_owned(),
            }],
            1,
            0,
        );
        assert_eq!(
            invalid
                .validate(&PreviewLimits::default())
                .expect_err("invalid integer component")
                .category(),
            "workspace.unsupportedValue"
        );
    }

    let all_zero_complex = preview(
        "int16",
        true,
        vec![PreviewValue::Integer {
            real: "7".to_owned(),
            imaginary: "0".to_owned(),
        }],
        1,
        0,
    );
    assert!(
        all_zero_complex
            .validate(&PreviewLimits::default())
            .is_err()
    );

    let numeric_component = r#"{
        "protocol":"openmat-kernel-v1","sessionId":"s","messageId":"r",
        "kind":"response","replyTo":"q","ok":true,
        "result":{"type":"inspect","data":{
            "class":"uint64","dimensions":[1,1],"complex":false,
            "selectedRange":{"start":[1,1],"size":[1,1]},
            "values":[{"kind":"integer","real":18446744073709551615,"imaginary":"0"}],
            "truncation":{"truncated":false,"omittedElements":0}
        }}
    }"#;
    assert!(matches!(
        decode_response(numeric_component, &PreviewLimits::default()),
        Err(CodecError::Json(_))
    ));
}

#[test]
fn floating_preview_validation_enforces_canonical_real_and_finite_complex_kinds() {
    for spelling in ["nan", "infinity", "negativeInfinity"] {
        preview(
            "double",
            false,
            vec![PreviewValue::Special {
                value: spelling.to_owned(),
            }],
            1,
            0,
        )
        .validate(&PreviewLimits::default())
        .expect("canonical real special");
    }
    let finite_complex = preview(
        "single",
        true,
        vec![PreviewValue::Complex {
            real: 1.0,
            imaginary: 0.0,
        }],
        1,
        0,
    );
    finite_complex
        .validate(&PreviewLimits::default())
        .expect("all complex elements use finite complex kind");

    for invalid in [
        preview(
            "double",
            false,
            vec![PreviewValue::Special {
                value: "NaN".to_owned(),
            }],
            1,
            0,
        ),
        preview(
            "double",
            true,
            vec![PreviewValue::Special {
                value: "nan".to_owned(),
            }],
            1,
            0,
        ),
        preview(
            "double",
            true,
            vec![PreviewValue::Complex {
                real: f64::NAN,
                imaginary: 0.0,
            }],
            1,
            0,
        ),
    ] {
        let response = response_with_preview(invalid);
        let error = encode_response(&response, &PreviewLimits::default())
            .expect_err("non-canonical or non-finite float preview");
        assert_eq!(
            error.validation_category(),
            Some("workspace.unsupportedValue")
        );
    }
}

#[test]
fn element_budget_accepts_exact_boundary_and_rejects_one_over() {
    let element_boundary = preview(
        "char",
        false,
        vec![PreviewValue::CharCodeUnit { value: 0 }; test_len(MAX_PREVIEW_ELEMENTS)],
        MAX_PREVIEW_ELEMENTS,
        0,
    );
    element_boundary
        .validate(&PreviewLimits::default())
        .expect("4096 elements fit");
    let element_over = preview(
        "char",
        false,
        vec![PreviewValue::CharCodeUnit { value: 0 }; test_len(MAX_PREVIEW_ELEMENTS + 1)],
        MAX_PREVIEW_ELEMENTS + 1,
        0,
    );
    assert_eq!(
        element_over
            .validate(&PreviewLimits::default())
            .expect_err("4097 elements exceed hard limit")
            .category(),
        "preview_bound"
    );
}

#[test]
fn string_budgets_accept_exact_boundaries_and_reject_one_over() {
    let per_element_boundary = preview(
        "string",
        false,
        vec![PreviewValue::String {
            code_units: vec![0; test_len(MAX_STRING_ELEMENT_CODE_UNITS)],
            missing: false,
        }],
        1,
        0,
    );
    per_element_boundary
        .validate(&PreviewLimits::default())
        .expect("16384 code units fit one element");
    let per_element_over = preview(
        "string",
        false,
        vec![PreviewValue::String {
            code_units: vec![0; test_len(MAX_STRING_ELEMENT_CODE_UNITS + 1)],
            missing: false,
        }],
        1,
        0,
    );
    assert_eq!(
        per_element_over
            .validate(&PreviewLimits::default())
            .expect_err("oversized element is unsupported")
            .category(),
        "workspace.unsupportedValue"
    );

    let aggregate_boundary = preview(
        "string",
        false,
        (0..4)
            .map(|_| PreviewValue::String {
                code_units: vec![0; test_len(MAX_STRING_ELEMENT_CODE_UNITS)],
                missing: false,
            })
            .collect(),
        4,
        0,
    );
    aggregate_boundary
        .validate(&PreviewLimits::default())
        .expect("65536 aggregate code units fit");
    let aggregate_over = preview(
        "string",
        false,
        vec![
            PreviewValue::String {
                code_units: vec![0; test_len(MAX_STRING_ELEMENT_CODE_UNITS)],
                missing: false,
            },
            PreviewValue::String {
                code_units: vec![0; test_len(MAX_STRING_ELEMENT_CODE_UNITS)],
                missing: false,
            },
            PreviewValue::String {
                code_units: vec![0; test_len(MAX_STRING_ELEMENT_CODE_UNITS)],
                missing: false,
            },
            PreviewValue::String {
                code_units: vec![0; test_len(MAX_STRING_ELEMENT_CODE_UNITS)],
                missing: false,
            },
            PreviewValue::String {
                code_units: vec![0],
                missing: false,
            },
        ],
        5,
        0,
    );
    assert_eq!(
        aggregate_over
            .validate(&PreviewLimits::default())
            .expect_err("65537 aggregate code units exceed hard limit")
            .category(),
        "preview_bound"
    );
}

#[test]
fn missing_elements_cost_zero_and_negotiated_budgets_are_enforced() {
    let limits = PreviewLimits::new(4, 2, 3).expect("small valid limits");
    let valid = preview(
        "string",
        false,
        vec![
            PreviewValue::String {
                code_units: vec![55_357, 0],
                missing: false,
            },
            PreviewValue::String {
                code_units: Vec::new(),
                missing: true,
            },
            PreviewValue::String {
                code_units: vec![65],
                missing: false,
            },
            PreviewValue::String {
                code_units: Vec::new(),
                missing: true,
            },
        ],
        4,
        0,
    );
    valid
        .validate(&limits)
        .expect("missing consumes zero code units");

    let invalid_missing = preview(
        "string",
        false,
        vec![PreviewValue::String {
            code_units: vec![0],
            missing: true,
        }],
        1,
        0,
    );
    assert_eq!(
        invalid_missing
            .validate(&limits)
            .expect_err("missing must have empty code units")
            .category(),
        "workspace.unsupportedValue"
    );

    let over_negotiated_per_element = preview(
        "string",
        false,
        vec![PreviewValue::String {
            code_units: vec![1, 2, 3],
            missing: false,
        }],
        1,
        0,
    );
    assert!(over_negotiated_per_element.validate(&limits).is_err());
}

#[test]
fn decoder_rejects_out_of_range_code_units_unknown_kinds_and_required_shapes() {
    let response_prefix = r#"{
        "protocol":"openmat-kernel-v1","sessionId":"s","messageId":"r",
        "kind":"response","replyTo":"q","ok":true,
        "result":{"type":"inspect","data":{
            "class":"char","dimensions":[1,1],"complex":false,
            "selectedRange":{"start":[1,1],"size":[1,1]},
            "values":["#;
    let response_suffix = r#"],
            "truncation":{"truncated":false,"omittedElements":0}
        }}
    }"#;
    for scalar in [
        r#"{"kind":"charCodeUnit","value":65536}"#,
        r#"{"kind":"charCodeUnit","value":-1}"#,
        r#"{"kind":"futureExactValue","value":1}"#,
    ] {
        let json = format!("{response_prefix}{scalar}{response_suffix}");
        assert!(matches!(
            decode_response(&json, &PreviewLimits::default()),
            Err(CodecError::Json(_))
        ));
    }

    let missing_complex =
        format!("{response_prefix}{{\"kind\":\"charCodeUnit\",\"value\":65}}{response_suffix}")
            .replace(",\"complex\":false", "");
    assert!(matches!(
        decode_response(&missing_complex, &PreviewLimits::default()),
        Err(CodecError::Json(_))
    ));
}

#[test]
fn structural_safe_integer_shape_and_truncation_rules_are_checked() {
    let empty_at_safe_limit = MatrixPreview {
        class: "char".to_owned(),
        dimensions: vec![MAX_SAFE_JSON_INTEGER, 0],
        complex: false,
        selected_range: MatrixRange {
            start: vec![MAX_SAFE_JSON_INTEGER, 1],
            size: vec![0, 0],
        },
        values: Vec::new(),
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
    };
    empty_at_safe_limit
        .validate(&PreviewLimits::default())
        .expect("maximum safe structural integer is accepted");

    let mut unsafe_dimension = empty_at_safe_limit.clone();
    unsafe_dimension.dimensions[0] = MAX_SAFE_JSON_INTEGER + 1;
    assert_eq!(
        unsafe_dimension
            .validate(&PreviewLimits::default())
            .expect_err("unsafe JSON dimension")
            .category(),
        "protocol.invalidMessage"
    );

    let rank_one = MatrixPreview {
        dimensions: vec![1],
        selected_range: MatrixRange {
            start: vec![1],
            size: vec![1],
        },
        ..preview(
            "char",
            false,
            vec![PreviewValue::CharCodeUnit { value: 0 }],
            1,
            0,
        )
    };
    assert!(rank_one.validate(&PreviewLimits::default()).is_err());

    let overflow_product = MatrixPreview {
        dimensions: vec![MAX_SAFE_JSON_INTEGER, MAX_SAFE_JSON_INTEGER],
        selected_range: MatrixRange {
            start: vec![1, 1],
            size: vec![0, 0],
        },
        values: Vec::new(),
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        class: "char".to_owned(),
        complex: false,
    };
    assert!(
        overflow_product
            .validate(&PreviewLimits::default())
            .is_err()
    );

    let out_of_bounds = MatrixPreview {
        dimensions: vec![1, 1],
        selected_range: MatrixRange {
            start: vec![1, 2],
            size: vec![1, 1],
        },
        ..preview(
            "char",
            false,
            vec![PreviewValue::CharCodeUnit { value: 0 }],
            1,
            0,
        )
    };
    assert!(out_of_bounds.validate(&PreviewLimits::default()).is_err());

    let truncated = preview(
        "char",
        false,
        vec![PreviewValue::CharCodeUnit { value: 0 }; test_len(MAX_PREVIEW_ELEMENTS)],
        MAX_PREVIEW_ELEMENTS + 1,
        1,
    );
    truncated
        .validate(&PreviewLimits::default())
        .expect("longest 4096-element prefix is valid");
    let mut inconsistent = truncated;
    inconsistent.truncation.omitted_elements = 2;
    assert_eq!(
        inconsistent
            .validate(&PreviewLimits::default())
            .expect_err("count and shape must agree")
            .category(),
        "preview_truncation"
    );
}

#[test]
fn request_and_response_validation_honor_the_request_element_limit() {
    let limits = PreviewLimits::new(10, 16, 64).expect("limits");
    assert!(inspect_request(11).validate(&limits).is_err());

    let three = preview(
        "char",
        false,
        vec![PreviewValue::CharCodeUnit { value: 0 }; 3],
        3,
        0,
    );
    three
        .validate(&limits)
        .expect("within negotiated element limit");
    assert_eq!(
        three
            .validate_for_request(&limits, 2)
            .expect_err("request maxElements is stricter")
            .category(),
        "preview_bound"
    );

    let request = RequestEnvelope::new(
        "session-1",
        "request-2",
        Request::Inspect(InspectRequest {
            name: "answer".to_owned(),
            range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 3],
            },
            max_elements: 2,
        }),
    );
    let response =
        ResponseEnvelope::success(&request, "response-2", ResponseResult::Inspect(three));
    assert_eq!(
        encode_response_for_request(&response, &request, &limits)
            .expect_err("codec enforces request maxElements")
            .validation_category(),
        Some("preview_bound")
    );

    let valid_preview = preview(
        "char",
        false,
        vec![PreviewValue::CharCodeUnit { value: 0 }; 2],
        3,
        1,
    );
    let valid_response = ResponseEnvelope::success(
        &request,
        "response-3",
        ResponseResult::Inspect(valid_preview),
    );
    let encoded = encode_response_for_request(&valid_response, &request, &limits)
        .expect("two-value prefix matches request limit");
    assert_eq!(
        decode_response_for_request(&encoded, &request, &limits)
            .expect("request-aware decoder")
            .reply_to,
        "request-2"
    );
}

#[test]
fn version_mixing_and_unknown_required_tags_fail_but_unknown_events_survive() {
    let request = RequestEnvelope::new("s", "q", Request::Interrupt(InterruptRequest {}));
    let mut request_json = serde_json::to_value(&request).expect("request JSON");
    request_json["protocol"] = Value::String(PROTOCOL_V0.to_owned());
    let error = decode_request(
        &serde_json::to_string(&request_json).expect("mixed JSON"),
        &PreviewLimits::default(),
    )
    .expect_err("v0 envelope is forbidden after v1 selection");
    assert_eq!(error.validation_category(), Some("protocol"));
    assert!(validate_session_protocol(ProtocolVersion::V1, PROTOCOL_V0).is_err());

    let initialize_in_v1 = r#"{
        "protocol":"openmat-kernel-v1","sessionId":"s","messageId":"q","kind":"request",
        "request":{"type":"initialize","params":{}}
    }"#;
    assert!(matches!(
        decode_request(initialize_in_v1, &PreviewLimits::default()),
        Err(CodecError::Json(_))
    ));
    let unknown_response_result = r#"{
        "protocol":"openmat-kernel-v1","sessionId":"s","messageId":"r","kind":"response",
        "replyTo":"q","ok":true,"result":{"type":"futureResult","data":{}}
    }"#;
    assert!(matches!(
        decode_response(unknown_response_result, &PreviewLimits::default()),
        Err(CodecError::Json(_))
    ));
    let unknown_message_kind = r#"{
        "protocol":"openmat-kernel-v1","sessionId":"s","messageId":"e","kind":"future",
        "event":{"type":"futureEvent","data":{}}
    }"#;
    assert!(matches!(
        decode_event(unknown_message_kind),
        Err(CodecError::Json(_))
    ));

    let unknown_event = r#"{
        "protocol":"openmat-kernel-v1","sessionId":"s","messageId":"e","kind":"event",
        "event":{"type":"futureEvent","data":{"answer":42}},"futureEnvelopeField":true
    }"#;
    let decoded = decode_event(unknown_event).expect("unknown event extension is allowed");
    assert!(matches!(decoded.event, Event::Unknown { .. }));
    let Event::Unknown { data, .. } = decoded.event else {
        unreachable!("checked unknown event")
    };
    assert_eq!(data["answer"], 42);
}

#[test]
fn v0_public_json_shapes_and_validation_remain_frozen() {
    let request = crate::RequestEnvelope::new(
        "session-1",
        "request-1",
        crate::Request::Initialize(crate::InitializeRequest {
            client: test_client(),
            supported_protocols: vec![PROTOCOL_V0.to_owned()],
            capabilities: crate::Capabilities::default(),
        }),
    );
    request.validate().expect("unchanged v0 request validates");
    let request_json = serde_json::to_value(&request).expect("serialize v0 request");
    assert_eq!(request_json["protocol"], PROTOCOL_V0);
    assert!(
        request_json["request"]["params"]["capabilities"]
            .get("maxStringElementCodeUnits")
            .is_none()
    );
    assert!(
        request_json["request"]["params"]["capabilities"]
            .get("maxPreviewCodeUnits")
            .is_none()
    );

    let preview = crate::MatrixPreview {
        class: "char".to_owned(),
        dimensions: vec![1, 1],
        selected_range: crate::MatrixRange {
            start: vec![1, 1],
            size: vec![1, 1],
        },
        values: vec![crate::PreviewValue::Text {
            value: "A".to_owned(),
        }],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
    };
    preview.validate().expect("v0 text remains accepted by v0");
    let preview_json = serde_json::to_value(&preview).expect("serialize v0 preview");
    assert!(preview_json.get("complex").is_none());
    assert_eq!(
        preview_json["values"][0],
        json!({"kind":"text", "value":"A"})
    );

    let mut wrong_protocol = request;
    wrong_protocol.protocol = PROTOCOL_V1.to_owned();
    assert_eq!(
        wrong_protocol
            .validate()
            .expect_err("v0 validator still rejects v1")
            .category(),
        "protocol"
    );
}

#[test]
fn malformed_bootstrap_versions_and_exchange_selection_are_rejected() {
    let request = bootstrap_request();
    let mut mixed_request = request.clone();
    mixed_request.protocol = PROTOCOL_V1.to_owned();
    assert_eq!(
        mixed_request
            .validate()
            .expect_err("initialize request must use v0 envelope")
            .category(),
        "protocol"
    );

    let response = BootstrapResponseEnvelope::success(
        &request,
        "kernel-1",
        InitializeResult {
            negotiated_protocol: "openmat-kernel-v9".to_owned(),
            implementation: test_kernel(),
            capabilities: Capabilities::default(),
        },
    );
    assert!(encode_bootstrap_response(&response).is_err());

    let failed = BootstrapResponseEnvelope::failure(
        &request,
        "kernel-2",
        ProtocolError::new("protocol.noCommonVersion", "no common protocol"),
    );
    failed.validate().expect("structured failure is valid");
    assert_eq!(failed.kind, MessageKind::Response);
    assert!(validate_initialize_exchange(&request, &failed).is_err());
}
