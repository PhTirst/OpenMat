use serde_json::{Value, json};

use super::*;
use crate::kernel_v1::{
    MAX_PREVIEW_CODE_UNITS, MAX_SAFE_JSON_INTEGER, MAX_STRING_ELEMENT_CODE_UNITS,
};
use crate::{
    Event, ExecutionMode, ImplementationInfo, InterruptRequest, MessageKind, PreviewTruncation,
    ProtocolError, StatusEvent,
};

fn test_client() -> ImplementationInfo {
    ImplementationInfo {
        name: "openmat-web".to_owned(),
        version: "2.0.0".to_owned(),
    }
}

fn test_kernel() -> ImplementationInfo {
    ImplementationInfo {
        name: "openmat-kernel".to_owned(),
        version: "2.0.0".to_owned(),
    }
}

fn v2_capabilities() -> Capabilities {
    Capabilities {
        execution_modes: vec![
            ExecutionMode::File,
            ExecutionMode::Cell,
            ExecutionMode::Repl,
        ],
        display_mime_types: vec!["text/plain".to_owned()],
        max_preview_elements: crate::MAX_PREVIEW_ELEMENTS,
        max_string_element_code_units: Some(MAX_STRING_ELEMENT_CODE_UNITS),
        max_preview_code_units: Some(MAX_PREVIEW_CODE_UNITS),
        max_aggregate_nodes: Some(MAX_AGGREGATE_NODES),
        max_aggregate_elements: Some(MAX_AGGREGATE_ELEMENTS),
        max_aggregate_depth: Some(MAX_AGGREGATE_DEPTH),
        interrupt: true,
        workspace_delta: true,
    }
}

fn bootstrap_request() -> BootstrapRequestEnvelope {
    BootstrapRequestEnvelope::new(
        "session-2",
        "client-1",
        InitializeRequest::v2(test_client(), v2_capabilities()),
    )
}

fn range(size: Vec<u64>) -> MatrixRange {
    MatrixRange {
        start: vec![1; size.len()],
        size,
    }
}

fn inspect_request(size: Vec<u64>, max_elements: u64) -> RequestEnvelope {
    RequestEnvelope::new(
        "session-2",
        "request-1",
        Request::Inspect(InspectRequest {
            name: "answer".to_owned(),
            range: range(size),
            max_elements,
        }),
    )
}

fn logical_scalar(value: bool) -> ExactValue {
    ExactValue {
        class: "logical".to_owned(),
        size: vec![1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        payload: ExactPayload::Logical {
            logical: vec![value],
        },
    }
}

fn numeric_scalar(value: &str) -> ExactValue {
    ExactValue {
        class: "double".to_owned(),
        size: vec![1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        payload: ExactPayload::Numeric {
            real: vec![value.to_owned()],
            imag: vec!["0".to_owned()],
        },
    }
}

fn empty_struct(fields: Vec<&str>, size: Vec<u64>) -> ExactValue {
    ExactValue {
        class: "struct".to_owned(),
        ndims: u64::try_from(size.len()).expect("test rank"),
        numel: 0,
        size,
        complex: false,
        payload: ExactPayload::Struct {
            fields: fields.into_iter().map(str::to_owned).collect(),
            records: Vec::new(),
        },
    }
}

fn response(preview: AggregatePreview, size: Vec<u64>) -> (RequestEnvelope, ResponseEnvelope) {
    let request = inspect_request(size, crate::MAX_PREVIEW_ELEMENTS);
    let response = ResponseEnvelope::success(
        &request,
        "response-1",
        ResponseResult::Inspect(InspectPreview::Aggregate(preview)),
    );
    (request, response)
}

#[test]
fn golden_v2_bootstrap_uses_v0_envelopes_and_exact_capability_shape() {
    let request = bootstrap_request();
    let encoded = encode_bootstrap_request(&request).expect("encode v2 bootstrap");
    let value: Value = serde_json::from_str(&encoded).expect("bootstrap JSON");
    assert_eq!(
        value,
        json!({
            "protocol": "openmat-kernel-v0",
            "sessionId": "session-2",
            "messageId": "client-1",
            "kind": "request",
            "request": {
                "type": "initialize",
                "params": {
                    "client": {"name": "openmat-web", "version": "2.0.0"},
                    "supportedProtocols": [
                        "openmat-kernel-v2", "openmat-kernel-v1", "openmat-kernel-v0"
                    ],
                    "capabilities": {
                        "executionModes": ["file", "cell", "repl"],
                        "displayMimeTypes": ["text/plain"],
                        "maxPreviewElements": 4096,
                        "maxStringElementCodeUnits": 16384,
                        "maxPreviewCodeUnits": 65536,
                        "maxAggregateNodes": 16384,
                        "maxAggregateElements": 65536,
                        "maxAggregateDepth": 32,
                        "interrupt": true,
                        "workspaceDelta": true
                    }
                }
            }
        })
    );
    assert_eq!(decode_bootstrap_request(&encoded).expect("decode"), request);

    let BootstrapRequest::Initialize(initialize) = &request.request;
    let negotiated = negotiate_initialize(initialize, &v2_capabilities()).expect("negotiate v2");
    assert_eq!(negotiated.protocol, ProtocolVersion::V2);
    let response = BootstrapResponseEnvelope::success(
        &request,
        "kernel-1",
        initialize_result(negotiated, test_kernel()),
    );
    let response_json = encode_bootstrap_response(&response).expect("encode response");
    let response_value: Value = serde_json::from_str(&response_json).expect("response JSON");
    assert_eq!(response_value["protocol"], crate::PROTOCOL_V0);
    assert_eq!(
        response_value["result"]["data"]["negotiatedProtocol"],
        PROTOCOL_V2
    );
    assert_eq!(
        validate_initialize_exchange(&request, &response).expect("valid exchange"),
        ProtocolVersion::V2
    );

    let idle = EventEnvelope::new(
        "session-2",
        "kernel-2",
        Event::Status(StatusEvent {
            status: crate::KernelStatus::Idle,
        }),
    );
    assert_eq!(idle.protocol, PROTOCOL_V2);
    idle.validate().expect("first v2 envelope");
}

#[test]
fn negotiation_downgrades_componentwise_and_strips_later_fields() {
    let kernel = v2_capabilities();
    let mut client = v2_capabilities();
    client.max_preview_elements = 100;
    client.max_string_element_code_units = Some(50);
    client.max_preview_code_units = Some(80);
    client.max_aggregate_nodes = Some(70);
    client.max_aggregate_elements = Some(60);
    client.max_aggregate_depth = Some(0);
    let negotiated = Capabilities::negotiate(&client, &kernel, ProtocolVersion::V2)
        .expect("componentwise v2 negotiation");
    assert_eq!(negotiated.max_preview_elements, 100);
    assert_eq!(negotiated.max_string_element_code_units, Some(50));
    assert_eq!(negotiated.max_preview_code_units, Some(80));
    assert_eq!(negotiated.max_aggregate_nodes, Some(70));
    assert_eq!(negotiated.max_aggregate_elements, Some(60));
    assert_eq!(negotiated.max_aggregate_depth, Some(0));

    let v1_request = InitializeRequest {
        client: test_client(),
        supported_protocols: vec![
            crate::kernel_v1::PROTOCOL_V1.to_owned(),
            crate::PROTOCOL_V0.to_owned(),
        ],
        capabilities: v2_capabilities(),
    };
    let v1 = negotiate_initialize(&v1_request, &kernel).expect("v1 downgrade");
    assert_eq!(v1.protocol, ProtocolVersion::V1);
    assert!(v1.capabilities.max_string_element_code_units.is_some());
    assert!(v1.capabilities.max_preview_code_units.is_some());
    assert_eq!(v1.capabilities.max_aggregate_nodes, None);
    assert_eq!(v1.capabilities.max_aggregate_elements, None);
    assert_eq!(v1.capabilities.max_aggregate_depth, None);
    let v1_envelope = BootstrapRequestEnvelope::new("s", "q1", v1_request);
    let v1_response = BootstrapResponseEnvelope::success(
        &v1_envelope,
        "r1",
        initialize_result(v1, test_kernel()),
    );
    let v1_json = encode_bootstrap_response(&v1_response).expect("v1 downgrade response");
    assert!(v1_json.contains("maxStringElementCodeUnits"));
    assert!(!v1_json.contains("maxAggregateNodes"));
    assert!(!v1_json.contains("maxAggregateElements"));
    assert!(!v1_json.contains("maxAggregateDepth"));

    let v0_request = InitializeRequest {
        client: test_client(),
        supported_protocols: vec![crate::PROTOCOL_V0.to_owned()],
        capabilities: v2_capabilities(),
    };
    let v0 = negotiate_initialize(&v0_request, &kernel).expect("v0 downgrade");
    assert_eq!(v0.protocol, ProtocolVersion::V0);
    assert_eq!(v0.capabilities.max_string_element_code_units, None);
    assert_eq!(v0.capabilities.max_preview_code_units, None);
    assert_eq!(v0.capabilities.max_aggregate_nodes, None);
    let v0_envelope = BootstrapRequestEnvelope::new("s", "q0", v0_request);
    let v0_response = BootstrapResponseEnvelope::success(
        &v0_envelope,
        "r0",
        initialize_result(v0, test_kernel()),
    );
    let v0_json = encode_bootstrap_response(&v0_response).expect("v0 downgrade response");
    assert!(!v0_json.contains("maxStringElementCodeUnits"));
    assert!(!v0_json.contains("maxPreviewCodeUnits"));
    assert!(!v0_json.contains("maxAggregateNodes"));
}

#[test]
fn invalid_v2_offer_is_fatal_instead_of_silent_downgrade() {
    let mut missing = InitializeRequest::v2(test_client(), v2_capabilities());
    missing.capabilities.max_aggregate_nodes = None;
    assert_eq!(
        negotiate_initialize(&missing, &v2_capabilities())
            .expect_err("missing v2 field")
            .category(),
        "protocol.invalidCapabilities"
    );

    let mut inconsistent = InitializeRequest::v2(test_client(), v2_capabilities());
    inconsistent.capabilities.max_string_element_code_units = Some(2);
    inconsistent.capabilities.max_preview_code_units = Some(1);
    assert_eq!(
        negotiate_initialize(&inconsistent, &v2_capabilities())
            .expect_err("inconsistent v1 limits in v2 offer")
            .category(),
        "protocol.invalidCapabilities"
    );

    let mut duplicate = InitializeRequest::v2(test_client(), v2_capabilities());
    duplicate.supported_protocols.push(PROTOCOL_V2.to_owned());
    assert_eq!(
        negotiate_initialize(&duplicate, &v2_capabilities())
            .expect_err("duplicate protocol")
            .category(),
        "protocol.invalidOffer"
    );
}

#[test]
fn canonical_nested_cell_and_shaped_empty_struct_match_spec_golden() {
    let preview = AggregatePreview::Cell {
        dimensions: vec![1, 2],
        selected_range: range(vec![1, 2]),
        items: vec![
            numeric_scalar("7"),
            empty_struct(vec!["beta", "alpha"], vec![0, 3]),
        ],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 3,
            elements: 3,
            code_units: 9,
            depth: 1,
        },
    };
    let (request, response) = response(preview, vec![1, 2]);
    let encoded = encode_response_for_request(&response, &request, &AggregateLimits::default())
        .expect("encode spec golden");
    let decoded = decode_response_for_request(&encoded, &request, &AggregateLimits::default())
        .expect("decode spec golden");
    assert_eq!(decoded, response);
    let value: Value = serde_json::from_str(&encoded).expect("JSON");
    let data = &value["result"]["data"];
    assert_eq!(data["class"], "cell");
    assert_eq!(data["items"][1]["size"], json!([0, 3]));
    assert_eq!(data["items"][1]["fields"], json!(["beta", "alpha"]));
    assert_eq!(
        data["usage"],
        json!({"nodes":3,"elements":3,"codeUnits":9,"depth":1})
    );
}

#[test]
fn shaped_empty_and_unfielded_scalar_structs_remain_distinct() {
    let shaped_empty = AggregatePreview::Struct {
        dimensions: vec![0, 3],
        selected_range: range(vec![0, 3]),
        fields: vec!["f".to_owned()],
        records: Vec::new(),
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 1,
            elements: 0,
            code_units: 1,
            depth: 0,
        },
    };
    shaped_empty
        .validate(&AggregateLimits::default())
        .expect("fielded shaped empty struct");

    let unfielded = ExactValue {
        class: "struct".to_owned(),
        size: vec![1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        payload: ExactPayload::Struct {
            fields: Vec::new(),
            records: vec![StructRecord::new(Vec::new())],
        },
    };
    let preview = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![unfielded],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 2,
            code_units: 0,
            depth: 1,
        },
    };
    let encoded = serde_json::to_string(&preview).expect("serialize unfielded struct");
    assert!(encoded.contains("\"records\":[{}]"));
    preview
        .validate(&AggregateLimits::default())
        .expect("unfielded scalar struct");
}

#[test]
fn struct_record_input_order_is_ignored_but_output_follows_fields() {
    let json = r#"{
        "class":"struct","dimensions":[1,1],"complex":false,
        "selectedRange":{"start":[1,1],"size":[1,1]},"kind":"struct",
        "fields":["beta","alpha"],
        "records":[{
            "alpha":{"class":"logical","size":[1,1],"ndims":2,"numel":1,"complex":false,"kind":"logical","logical":[false]},
            "beta":{"class":"logical","size":[1,1],"ndims":2,"numel":1,"complex":false,"kind":"logical","logical":[true]}
        }],
        "truncation":{"truncated":false,"omittedElements":0},
        "usage":{"nodes":3,"elements":3,"codeUnits":9,"depth":1}
    }"#;
    let preview: AggregatePreview = serde_json::from_str(json).expect("decode reordered members");
    preview
        .validate(&AggregateLimits::default())
        .expect("member order is semantic-free");
    let encoded = serde_json::to_string(&preview).expect("canonical producer order");
    let records = encoded
        .split("\"records\":")
        .nth(1)
        .expect("records suffix");
    assert!(records.find("\"beta\"").expect("beta") < records.find("\"alpha\"").expect("alpha"));
}

#[test]
fn whole_top_level_truncation_is_valid_and_nested_truncation_is_not() {
    let limits =
        AggregateLimits::new(4096, 16_384, 65_536, 16_384, 2, 32).expect("small element limit");
    let preview = AggregatePreview::Cell {
        dimensions: vec![1, 2],
        selected_range: range(vec![1, 2]),
        items: vec![logical_scalar(true)],
        truncation: PreviewTruncation {
            truncated: true,
            omitted_elements: 1,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 2,
            code_units: 0,
            depth: 1,
        },
    };
    preview.validate(&limits).expect("whole-element prefix");

    let malformed = r#"{
        "class":"cell","size":[1,2],"ndims":2,"numel":2,"complex":false,
        "kind":"cell","items":[],
        "truncation":{"truncated":true,"omittedElements":2}
    }"#;
    assert!(serde_json::from_str::<ExactValue>(malformed).is_err());
}

#[test]
fn every_exact_variant_uses_canonical_schema_v2_shape() {
    let values = vec![
        ExactValue {
            class: "double".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: true,
            payload: ExactPayload::Numeric {
                real: vec!["NaN".to_owned()],
                imag: vec!["-0.1e+2".to_owned()],
            },
        },
        ExactValue {
            class: "uint64".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            payload: ExactPayload::Integer {
                integer: vec![IntegerValue {
                    real: u64::MAX.to_string(),
                    imaginary: "0".to_owned(),
                }],
            },
        },
        logical_scalar(true),
        ExactValue {
            class: "char".to_owned(),
            size: vec![1, 2],
            ndims: 2,
            numel: 2,
            complex: false,
            payload: ExactPayload::Char {
                code_units: vec![55_357, 56_898],
            },
        },
        ExactValue {
            class: "string".to_owned(),
            size: vec![1, 2],
            ndims: 2,
            numel: 2,
            complex: false,
            payload: ExactPayload::String {
                string_code_units: vec![vec![65], Vec::new()],
                missing: vec![false, true],
            },
        },
        ExactValue {
            class: "struct".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            payload: ExactPayload::Struct {
                fields: Vec::new(),
                records: vec![StructRecord::new(Vec::new())],
            },
        },
    ];
    let exact_cell = ExactValue {
        class: "cell".to_owned(),
        size: vec![1, 6],
        ndims: 2,
        numel: 6,
        complex: false,
        payload: ExactPayload::Cell { items: values },
    };
    let preview = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![exact_cell],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 8,
            elements: 15,
            code_units: 3,
            depth: 2,
        },
    };
    preview
        .validate(&AggregateLimits::default())
        .expect("all accepted exact variants");
    let encoded = serde_json::to_string(&preview).expect("canonical exact JSON");
    let decoded: AggregatePreview = serde_json::from_str(&encoded).expect("decode exact JSON");
    assert_eq!(decoded, preview);
}

#[test]
fn numeric_integer_string_and_payload_grammar_failures_are_rejected() {
    for invalid in ["nan", "Inf", "+1", "01", "1.", ".1", "1e", " 1"] {
        let mut value = numeric_scalar(invalid);
        let preview = AggregatePreview::Cell {
            dimensions: vec![1, 1],
            selected_range: range(vec![1, 1]),
            items: vec![value.clone()],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: PreviewUsage {
                nodes: 2,
                elements: 2,
                code_units: 0,
                depth: 1,
            },
        };
        assert!(
            preview.validate(&AggregateLimits::default()).is_err(),
            "{invalid}"
        );
        value.complex = true;
    }

    let invalid_integer = ExactValue {
        class: "int8".to_owned(),
        size: vec![1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        payload: ExactPayload::Integer {
            integer: vec![IntegerValue {
                real: "128".to_owned(),
                imaginary: "0".to_owned(),
            }],
        },
    };
    let invalid_string = ExactValue {
        class: "string".to_owned(),
        size: vec![1, 1],
        ndims: 2,
        numel: 1,
        complex: false,
        payload: ExactPayload::String {
            string_code_units: vec![vec![65]],
            missing: vec![true],
        },
    };
    for value in [invalid_integer, invalid_string] {
        let preview = AggregatePreview::Cell {
            dimensions: vec![1, 1],
            selected_range: range(vec![1, 1]),
            items: vec![value],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: PreviewUsage {
                nodes: 2,
                elements: 2,
                code_units: 0,
                depth: 1,
            },
        };
        assert!(preview.validate(&AggregateLimits::default()).is_err());
    }

    let mismatched = ExactValue {
        class: "logical".to_owned(),
        size: vec![1, 2],
        ndims: 2,
        numel: 2,
        complex: false,
        payload: ExactPayload::Logical {
            logical: vec![true],
        },
    };
    let preview = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![mismatched],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 3,
            code_units: 0,
            depth: 1,
        },
    };
    assert_eq!(
        preview
            .validate(&AggregateLimits::default())
            .expect_err("payload count mismatch")
            .category(),
        "engine.invalidPreview"
    );
}

#[test]
fn unknown_exact_preview_request_and_result_kinds_fail() {
    for exact in [
        r#"{"class":"ExampleHandle","size":[1,1],"ndims":2,"numel":1,"complex":false,"kind":"object","fields":[],"records":[{}]}"#,
        r#"{"class":"Nothing","size":[1,1],"ndims":2,"numel":1,"complex":false,"kind":"Nothing"}"#,
        r#"{"class":"logical","size":[1,1],"ndims":2,"numel":1,"complex":false,"kind":"future","logical":[true]}"#,
    ] {
        assert!(serde_json::from_str::<ExactValue>(exact).is_err());
    }

    let unknown_request = r#"{
        "protocol":"openmat-kernel-v2","sessionId":"s","messageId":"q","kind":"request",
        "request":{"type":"futureRequest","params":{}}
    }"#;
    assert!(matches!(
        decode_request(unknown_request, &AggregateLimits::default()),
        Err(CodecError::Json(_))
    ));
    let unknown_result = r#"{
        "protocol":"openmat-kernel-v2","sessionId":"s","messageId":"r","kind":"response",
        "replyTo":"q","ok":true,"result":{"type":"futureResult","data":{}}
    }"#;
    assert!(matches!(
        decode_response(unknown_result, &AggregateLimits::default()),
        Err(CodecError::Json(_))
    ));
    let unknown_preview = r#"{
        "protocol":"openmat-kernel-v2","sessionId":"s","messageId":"r","kind":"response",
        "replyTo":"q","ok":true,"result":{"type":"inspect","data":{
            "class":"cell","dimensions":[0,0],"complex":false,
            "selectedRange":{"start":[1,1],"size":[0,0]},"kind":"future",
            "items":[],"values":[],"truncation":{"truncated":false,"omittedElements":0},
            "usage":{"nodes":1,"elements":0,"codeUnits":0,"depth":0}
        }}
    }"#;
    assert!(matches!(
        decode_response(unknown_preview, &AggregateLimits::default()),
        Err(CodecError::Json(_))
    ));
}

#[test]
fn field_schemas_and_record_key_sets_are_exact() {
    let record = StructRecord::new(vec![("beta".to_owned(), logical_scalar(true))]);
    let malformed = AggregatePreview::Struct {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        fields: vec!["beta".to_owned(), "alpha".to_owned()],
        records: vec![record],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 2,
            code_units: 9,
            depth: 1,
        },
    };
    assert_eq!(
        malformed
            .validate(&AggregateLimits::default())
            .expect_err("missing alpha")
            .category(),
        "engine.invalidPreview"
    );

    for fields in [vec!["valid", "valid"], vec!["alpha", "α"], vec!["_bad"]] {
        let exact = empty_struct(fields, vec![0, 0]);
        let preview = AggregatePreview::Cell {
            dimensions: vec![1, 1],
            selected_range: range(vec![1, 1]),
            items: vec![exact],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: PreviewUsage {
                nodes: 2,
                elements: 1,
                code_units: 0,
                depth: 1,
            },
        };
        assert!(preview.validate(&AggregateLimits::default()).is_err());
    }

    let duplicate_record_member = r#"{
        "class":"struct","dimensions":[1,1],"complex":false,
        "selectedRange":{"start":[1,1],"size":[1,1]},"kind":"struct",
        "fields":["f"],
        "records":[{
            "f":{"class":"logical","size":[1,1],"ndims":2,"numel":1,"complex":false,"kind":"logical","logical":[true]},
            "f":{"class":"logical","size":[1,1],"ndims":2,"numel":1,"complex":false,"kind":"logical","logical":[false]}
        }],
        "truncation":{"truncated":false,"omittedElements":0},
        "usage":{"nodes":2,"elements":2,"codeUnits":1,"depth":1}
    }"#;
    assert!(serde_json::from_str::<AggregatePreview>(duplicate_record_member).is_err());
}

#[test]
fn usage_is_recomputed_and_all_semantic_boundaries_are_enforced() {
    let mut preview = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![logical_scalar(true)],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 2,
            code_units: 0,
            depth: 1,
        },
    };
    preview
        .validate(&AggregateLimits::default())
        .expect("exact usage boundary");
    let AggregatePreview::Cell { usage, .. } = &mut preview else {
        unreachable!()
    };
    usage.nodes = 1;
    assert_eq!(
        preview
            .validate(&AggregateLimits::default())
            .expect_err("usage is recomputed")
            .category(),
        "engine.invalidPreview"
    );

    let preview = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![logical_scalar(true)],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 2,
            code_units: 0,
            depth: 1,
        },
    };
    for limits in [
        AggregateLimits::new(4096, 1, 1, 1, 2, 1).expect("node limit"),
        AggregateLimits::new(4096, 1, 1, 2, 1, 1).expect("element limit"),
    ] {
        assert_eq!(
            preview
                .validate(&limits)
                .expect_err("cumulative limit")
                .category(),
            "workspace.previewLimit"
        );
    }

    let char_preview = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![ExactValue {
            class: "char".to_owned(),
            size: vec![1, 2],
            ndims: 2,
            numel: 2,
            complex: false,
            payload: ExactPayload::Char {
                code_units: vec![1, 2],
            },
        }],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 3,
            code_units: 2,
            depth: 1,
        },
    };
    let code_limit = AggregateLimits::new(4096, 1, 1, 2, 3, 1).expect("code limit");
    assert_eq!(
        char_preview
            .validate(&code_limit)
            .expect_err("code units over")
            .category(),
        "workspace.previewLimit"
    );
}

#[test]
fn per_node_per_string_and_depth_limits_are_hard_failures() {
    let node_over = ExactValue {
        class: "logical".to_owned(),
        size: vec![1, 2],
        ndims: 2,
        numel: 2,
        complex: false,
        payload: ExactPayload::Logical {
            logical: vec![true, false],
        },
    };
    let preview = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![node_over],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 3,
            code_units: 0,
            depth: 1,
        },
    };
    let limits = AggregateLimits::new(1, 1, 2, 4, 4, 4).expect("per node limit");
    assert_eq!(
        preview
            .validate(&limits)
            .expect_err("per-node numel")
            .category(),
        "workspace.previewLimit"
    );

    let string_over = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![ExactValue {
            class: "string".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            payload: ExactPayload::String {
                string_code_units: vec![vec![1, 2]],
                missing: vec![false],
            },
        }],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 2,
            code_units: 2,
            depth: 1,
        },
    };
    let limits = AggregateLimits::new(2, 1, 2, 4, 4, 4).expect("per string limit");
    assert_eq!(
        string_over
            .validate(&limits)
            .expect_err("per-string units")
            .category(),
        "workspace.previewLimit"
    );

    let mut nested = logical_scalar(true);
    for _ in 0..4 {
        nested = ExactValue {
            class: "cell".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            payload: ExactPayload::Cell {
                items: vec![nested],
            },
        };
    }
    let deep = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: range(vec![1, 1]),
        items: vec![nested],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 6,
            elements: 6,
            code_units: 0,
            depth: 5,
        },
    };
    let limits = AggregateLimits::new(8, 1, 8, 8, 8, 3).expect("depth limit");
    assert_eq!(
        deep.validate(&limits).expect_err("depth over").category(),
        "workspace.previewDepth"
    );
}

#[test]
fn active_path_guard_rejects_cycles_before_depth_and_allows_sibling_sharing() {
    let mut guard = ActivePathGuard::default();
    guard.enter(7_u64, 1, 32).expect("enter root identity");
    assert_eq!(
        guard
            .enter(7, 100, 32)
            .expect_err("cycle has precedence over depth")
            .category(),
        "workspace.cyclicValue"
    );
    guard.leave(&7).expect("balanced leave");
    guard.enter(7, 1, 32).expect("sibling sharing is legal");
    guard.leave(&7).expect("leave sibling");
    assert!(guard.is_empty());

    assert_eq!(
        guard
            .enter(9, 33, 32)
            .expect_err("depth boundary")
            .category(),
        "workspace.previewDepth"
    );
}

#[test]
fn safe_integer_shape_product_selected_range_and_truncation_are_checked() {
    let empty = AggregatePreview::Cell {
        dimensions: vec![MAX_SAFE_JSON_INTEGER, 0],
        selected_range: MatrixRange {
            start: vec![MAX_SAFE_JSON_INTEGER, 1],
            size: vec![0, 0],
        },
        items: Vec::new(),
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 1,
            elements: 0,
            code_units: 0,
            depth: 0,
        },
    };
    empty
        .validate(&AggregateLimits::default())
        .expect("safe shaped empty");

    let overflow = AggregatePreview::Cell {
        dimensions: vec![MAX_SAFE_JSON_INTEGER, 2],
        selected_range: range(vec![0, 0]),
        items: Vec::new(),
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 1,
            elements: 0,
            code_units: 0,
            depth: 0,
        },
    };
    assert!(overflow.validate(&AggregateLimits::default()).is_err());

    let out_of_bounds = AggregatePreview::Cell {
        dimensions: vec![1, 1],
        selected_range: MatrixRange {
            start: vec![1, 2],
            size: vec![1, 1],
        },
        items: vec![logical_scalar(true)],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 2,
            elements: 2,
            code_units: 0,
            depth: 1,
        },
    };
    assert!(out_of_bounds.validate(&AggregateLimits::default()).is_err());

    let mut bad_truncation = empty;
    let AggregatePreview::Cell { truncation, .. } = &mut bad_truncation else {
        unreachable!()
    };
    truncation.truncated = true;
    assert_eq!(
        bad_truncation
            .validate(&AggregateLimits::default())
            .expect_err("canonical truncation Boolean")
            .category(),
        "preview_truncation"
    );
}

#[test]
fn request_aware_codec_accepts_v1_matrix_shape_and_rejects_protocol_mixing() {
    let request = inspect_request(vec![1, 1], 1);
    let matrix = MatrixPreview {
        class: "char".to_owned(),
        dimensions: vec![1, 1],
        complex: false,
        selected_range: range(vec![1, 1]),
        values: vec![PreviewValue::CharCodeUnit { value: 65 }],
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
    };
    let response = ResponseEnvelope::success(
        &request,
        "response-matrix",
        ResponseResult::Inspect(InspectPreview::Matrix(matrix)),
    );
    let encoded = encode_response_for_request(&response, &request, &AggregateLimits::default())
        .expect("v1-exact matrix JSON in v2 envelope");
    assert!(!encoded.contains("\"kind\":\"char\""));
    decode_response_for_request(&encoded, &request, &AggregateLimits::default())
        .expect("request-aware matrix decode");

    let mixed = encoded.replacen(PROTOCOL_V2, crate::kernel_v1::PROTOCOL_V1, 1);
    assert_eq!(
        decode_response(&mixed, &AggregateLimits::default())
            .expect_err("v1 envelope in v2 session")
            .validation_category(),
        Some("protocol")
    );
    assert!(validate_session_protocol(ProtocolVersion::V2, crate::PROTOCOL_V0).is_err());

    let wrong_range_request = inspect_request(vec![1, 2], 1);
    assert_eq!(
        decode_response_for_request(&encoded, &wrong_range_request, &AggregateLimits::default())
            .expect_err("selected range mismatch")
            .validation_category(),
        Some("response_shape")
    );
}

#[test]
fn unknown_events_survive_while_known_event_shapes_remain_checked() {
    let json = r#"{
        "protocol":"openmat-kernel-v2","sessionId":"s","messageId":"e","kind":"event",
        "event":{"type":"futureEvent","data":{"answer":42}},"futureEnvelopeField":true
    }"#;
    let decoded = decode_event(json).expect("unknown event extension");
    let Event::Unknown { event_type, data } = decoded.event else {
        panic!("future event must remain unknown")
    };
    assert_eq!(event_type, "futureEvent");
    assert_eq!(data["answer"], 42);

    let request = RequestEnvelope::new("s", "q", Request::Interrupt(InterruptRequest {}));
    let failure = ResponseEnvelope::failure(
        &request,
        "r",
        ProtocolError::new("workspace.unsupportedValue", "aggregate unavailable"),
    );
    encode_response_for_request(&failure, &request, &AggregateLimits::default())
        .expect("structured failure remains valid");
}

#[test]
fn checked_complete_frame_helper_accepts_exact_limit_and_rejects_one_over() {
    let mut exact = vec![b' '; MAX_JSON_FRAME_BYTES];
    exact[0] = b'[';
    exact[1] = b']';
    check_json_frame_bytes(&exact).expect("exactly one MiB complete JSON text");

    let mut over = vec![b' '; MAX_JSON_FRAME_BYTES + 1];
    over[0] = b'[';
    over[1] = b']';
    assert!(matches!(
        check_json_frame_bytes(&over),
        Err(CodecError::FrameTooLarge {
            actual,
            maximum: MAX_JSON_FRAME_BYTES
        }) if actual == MAX_JSON_FRAME_BYTES + 1
    ));
    assert!(matches!(
        check_json_frame_bytes(b"\xef\xbb\xbf{}"),
        Err(CodecError::Utf8Bom)
    ));
    assert!(matches!(
        check_json_frame_bytes(&[0xff]),
        Err(CodecError::Utf8(_))
    ));
}

#[test]
fn v0_and_v1_public_models_and_goldens_remain_unchanged() {
    let v0 = crate::RequestEnvelope::new("s", "q0", crate::Request::Interrupt(InterruptRequest {}));
    v0.validate().expect("frozen v0 request");
    let v0_json = serde_json::to_string(&v0).expect("v0 JSON");
    assert_eq!(
        serde_json::from_str::<Value>(&v0_json).expect("v0 value")["protocol"],
        crate::PROTOCOL_V0
    );

    let v1 = crate::kernel_v1::RequestEnvelope::new(
        "s",
        "q1",
        crate::kernel_v1::Request::Interrupt(InterruptRequest {}),
    );
    v1.validate(&crate::kernel_v1::PreviewLimits::default())
        .expect("frozen v1 request");
    let v1_json =
        crate::kernel_v1::encode_request(&v1, &crate::kernel_v1::PreviewLimits::default())
            .expect("v1 JSON");
    assert_eq!(
        serde_json::from_str::<Value>(&v1_json).expect("v1 value")["protocol"],
        crate::kernel_v1::PROTOCOL_V1
    );
    assert!(!v1_json.contains("maxAggregateNodes"));
}

#[test]
fn aggregate_payload_under_v0_or_v1_is_not_reinterpreted() {
    let aggregate_data = r#"{
        "class":"cell","dimensions":[0,0],"complex":false,
        "selectedRange":{"start":[1,1],"size":[0,0]},"kind":"cell","items":[],
        "truncation":{"truncated":false,"omittedElements":0},
        "usage":{"nodes":1,"elements":0,"codeUnits":0,"depth":0}
    }"#;
    let v1 = format!(
        "{{\"protocol\":\"openmat-kernel-v1\",\"sessionId\":\"s\",\"messageId\":\"r\",\"kind\":\"response\",\"replyTo\":\"q\",\"ok\":true,\"result\":{{\"type\":\"inspect\",\"data\":{aggregate_data}}}}}"
    );
    assert!(
        crate::kernel_v1::decode_response(&v1, &crate::kernel_v1::PreviewLimits::default())
            .is_err()
    );

    let v0 = format!(
        "{{\"protocol\":\"openmat-kernel-v0\",\"sessionId\":\"s\",\"messageId\":\"r\",\"kind\":\"response\",\"replyTo\":\"q\",\"ok\":true,\"result\":{{\"type\":\"inspect\",\"data\":{aggregate_data}}}}}"
    );
    assert!(serde_json::from_str::<crate::ResponseEnvelope>(&v0).is_err());
}

#[test]
fn rust_enum_layout_is_not_treated_as_wire_abi() {
    let preview = InspectPreview::Aggregate(AggregatePreview::Cell {
        dimensions: vec![0, 0],
        selected_range: range(vec![0, 0]),
        items: Vec::new(),
        truncation: PreviewTruncation {
            truncated: false,
            omitted_elements: 0,
        },
        usage: PreviewUsage {
            nodes: 1,
            elements: 0,
            code_units: 0,
            depth: 0,
        },
    });
    let json = serde_json::to_value(&preview).expect("wire shape");
    assert_eq!(json["kind"], "cell");
    assert!(json.get("Aggregate").is_none());
    assert!(json.get("Cell").is_none());
    assert_eq!(MessageKind::Response, MessageKind::Response);
}
