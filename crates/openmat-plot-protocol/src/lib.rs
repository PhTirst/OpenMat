#![doc = "Versioned `OpenMat` graphics control messages and binary data framing."]
#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)] // Every fallible public API returns typed ValidationError.

mod binary;
mod data;
mod error;
mod limits;
mod message;
mod scene;

pub use binary::{
    BINARY_HEADER_LIMIT, BINARY_PREFIX_LEN, BufferTransferAssembler, CompletedBuffer,
    DecodedBinaryFrame, OMGP_MAGIC, PROTOCOL_MAJOR, PROTOCOL_MINOR, decode_binary_frame,
    encode_binary_frame,
};
pub use data::{BufferChunkHeader, BufferTransfer, DataRef, DataType, Endianness, StorageOrder};
pub use error::{ErrorCategory, ProtocolFailure, ValidationError};
pub use limits::{
    ClientCapabilities, GraphicsLimits, MAX_BINARY_FRAME_BYTES, MAX_BUFFER_BYTES, MAX_LIVE_OBJECTS,
    MAX_RESIDENT_BYTES, MAX_SAFE_INTEGER, MAX_TEXT_FRAME_BYTES, RenderBackend,
};
pub use message::{
    AttachToken, BufferReleasedEvent, ClientInfo, CloseFigureRequest, CloseFigureResult,
    DecodedRequest, Event, EventEnvelope, FigureClosedEvent, FigureSummary, GetBufferRequest,
    GetSnapshotRequest, GetSnapshotResult, GraphicsProtocol, ImplementationInfo, InitializeRequest,
    InitializeResult, MessageKind, PROTOCOL, PROTOCOL_V1, PROTOCOL_V2, PROTOCOL_V3, PROTOCOL_V4,
    ReleaseBufferRequest, ReleaseBufferResult, Request, RequestEnvelope, ResponseEnvelope,
    ResyncFigureRequest, ResyncFigureResult, ServerCapabilities, SessionClosedEvent,
    SetAxesCameraRequest, SetAxesCameraResult, SetAxesLimitsRequest, SetAxesLimitsResult,
    ShutdownRequest, ShutdownResult, UnsupportedRequest, decode_event, decode_event_for,
    decode_request, decode_request_for, encode_event, encode_response,
};
pub use scene::{
    Axes2DProperties, AxesCoordinateSystem, AxesLimitMode, AxesTickMode, AxisDirection,
    CDataMapping, ChartColorMode, ChartColorProperties, ChartGroupProperties, ChartType,
    ColorBarProperties, DeltaOperation, FigureDelta, FigureNextPlot, FigureProperties,
    FigureSnapshot, FontStyle, GraphicsObject, HorizontalAlignment, LegendLocation,
    LegendOrientation, LegendProperties, LineSeriesProperties, LineStyle, Marker, MarkerIndex,
    NextPlot, Nullable, ObjectFields, PatchSeriesProperties, Projection, Rgba, Scale,
    ScatterColorTarget, ScatterSeriesProperties, SceneState, SurfaceColorMode,
    SurfaceSeriesProperties, TextInterpreter, TextProperties, TextRole, ThetaAxisUnits,
    ThetaDirection, ThetaZeroLocation, TickDirection, TickValue, VerticalAlignment,
};
