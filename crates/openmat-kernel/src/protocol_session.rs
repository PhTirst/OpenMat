use std::error::Error;
use std::fmt;

use openmat_protocol::kernel_v1::{self, PreviewLimits};
use openmat_protocol::kernel_v2::{self, AggregateLimits, ProtocolVersion};
use openmat_protocol::kernel_v3;
use openmat_protocol::{
    Event, KernelStatus, ProtocolError, RequestEnvelope, ServerMessage, ShutdownResult,
    WorkspaceSummary,
};

use crate::{ExecutionEngine, ExecutionOutput, Kernel, KernelControl};

#[derive(Clone, Copy, Debug)]
struct NegotiatedSession {
    protocol: ProtocolVersion,
    preview_limits: Option<PreviewLimits>,
    aggregate_limits: Option<AggregateLimits>,
}

/// One typed request accepted by [`KernelSession`].
#[derive(Clone, Copy, Debug)]
pub enum KernelSessionRequest<'a> {
    /// The only legal request before protocol negotiation.
    Bootstrap(&'a kernel_v2::BootstrapRequestEnvelope),
    /// A post-bootstrap v0 request.
    V0(&'a RequestEnvelope),
    /// A post-bootstrap v1 request.
    V1(&'a kernel_v1::RequestEnvelope),
    /// A post-bootstrap v2 request.
    V2(&'a kernel_v2::RequestEnvelope),
    /// A post-bootstrap v3 request.
    V3(&'a kernel_v3::RequestEnvelope),
}

/// One typed server message emitted by [`KernelSession`].
#[derive(Clone, Debug, PartialEq)]
pub enum KernelSessionMessage {
    /// The final bootstrap-v0 initialize response.
    Bootstrap(kernel_v2::BootstrapResponseEnvelope),
    /// A post-bootstrap v0 response or event.
    V0(ServerMessage),
    /// A post-bootstrap v1 response or event.
    V1(kernel_v1::ServerMessage),
    /// A post-bootstrap v2 response or event.
    V2(kernel_v2::ServerMessage),
    /// A post-bootstrap v3 response or event.
    V3(kernel_v3::ServerMessage),
}

/// A negotiated kernel protocol session.
///
/// Before initialization this entry accepts only a bootstrap-v0 initialize
/// request. After a successful response it locks the selected protocol. The
/// v1 raw-frame helper always uses the request-aware response codec.
pub struct KernelSession<E> {
    kernel: Kernel<E>,
    negotiated: Option<NegotiatedSession>,
}

impl<E: ExecutionEngine> KernelSession<E> {
    /// Creates an uninitialized session in the `starting` state.
    #[must_use]
    pub fn new(session_id: impl Into<String>, engine: E) -> Self {
        Self {
            kernel: Kernel::new(session_id, engine),
            negotiated: None,
        }
    }

    /// Returns the current kernel lifecycle state.
    #[must_use]
    pub fn status(&self) -> KernelStatus {
        self.kernel.status()
    }

    /// Returns the selected protocol after successful initialization.
    #[must_use]
    pub fn negotiated_protocol(&self) -> Option<ProtocolVersion> {
        self.negotiated.map(|negotiated| negotiated.protocol)
    }

    /// Creates an independent interrupt path after protocol negotiation.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] if initialization has not succeeded.
    pub fn control(&self) -> Result<KernelSessionControl, KernelSessionError> {
        let negotiated = self.negotiated.ok_or_else(|| {
            KernelSessionError::new(
                "kernel.notInitialized",
                "initialize must succeed before creating a session control path",
            )
        })?;
        Ok(KernelSessionControl {
            control: self.kernel.control(),
            negotiated,
        })
    }

    /// Handles one typed request and returns ordered typed response/event data.
    ///
    /// The successful bootstrap response is always the first returned message;
    /// its immediately following idle event already uses the selected protocol.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] for an illegal pre-initialize request,
    /// protocol mixing, or an invalid engine-produced v1 message.
    #[allow(clippy::too_many_lines)] // The exhaustive protocol-state match is clearest in one place.
    pub fn handle(
        &mut self,
        request: KernelSessionRequest<'_>,
    ) -> Result<Vec<KernelSessionMessage>, KernelSessionError> {
        match (self.negotiated, request) {
            (None, KernelSessionRequest::Bootstrap(request)) => self.handle_bootstrap(request),
            (
                None,
                KernelSessionRequest::V0(_)
                | KernelSessionRequest::V1(_)
                | KernelSessionRequest::V2(_)
                | KernelSessionRequest::V3(_),
            ) => Err(KernelSessionError::new(
                "protocol.bootstrapRequired",
                "only bootstrap initialize is legal before protocol negotiation",
            )),
            (Some(_), KernelSessionRequest::Bootstrap(_)) => Err(KernelSessionError::new(
                "protocol.alreadyNegotiated",
                "bootstrap initialize is forbidden after protocol negotiation",
            )),
            (
                Some(NegotiatedSession {
                    protocol: ProtocolVersion::V0,
                    ..
                }),
                KernelSessionRequest::V0(request),
            ) => {
                validate_typed_protocol(ProtocolVersion::V0, &request.protocol)?;
                Ok(self
                    .kernel
                    .handle_request(request)
                    .into_iter()
                    .map(KernelSessionMessage::V0)
                    .collect())
            }
            (
                Some(NegotiatedSession {
                    protocol: ProtocolVersion::V1,
                    preview_limits: Some(limits),
                    ..
                }),
                KernelSessionRequest::V1(request),
            ) => {
                validate_typed_protocol(ProtocolVersion::V1, &request.protocol)?;
                let messages = self.kernel.handle_v1_request(request, &limits);
                validate_v1_messages(&messages, request, &limits)?;
                Ok(messages.into_iter().map(KernelSessionMessage::V1).collect())
            }
            (
                Some(NegotiatedSession {
                    protocol: ProtocolVersion::V1,
                    preview_limits: None,
                    ..
                }),
                KernelSessionRequest::V1(_),
            ) => Err(KernelSessionError::new(
                "protocol.invalidNegotiation",
                "v1 session is missing negotiated preview limits",
            )),
            (
                Some(NegotiatedSession {
                    protocol: ProtocolVersion::V2,
                    aggregate_limits: Some(limits),
                    ..
                }),
                KernelSessionRequest::V2(request),
            ) => {
                validate_typed_protocol(ProtocolVersion::V2, &request.protocol)?;
                let messages = self.kernel.handle_v2_request(request, &limits);
                validate_v2_messages(&messages, request, &limits)?;
                Ok(messages.into_iter().map(KernelSessionMessage::V2).collect())
            }
            (
                Some(NegotiatedSession {
                    protocol: ProtocolVersion::V3,
                    aggregate_limits: Some(limits),
                    ..
                }),
                KernelSessionRequest::V3(request),
            ) => {
                validate_typed_protocol(ProtocolVersion::V3, &request.protocol)?;
                let messages = self.kernel.handle_v3_request(request, &limits);
                validate_v3_messages(&messages, request, &limits)?;
                Ok(messages.into_iter().map(KernelSessionMessage::V3).collect())
            }
            (
                Some(NegotiatedSession {
                    protocol: ProtocolVersion::V2,
                    aggregate_limits: None,
                    ..
                }),
                KernelSessionRequest::V2(_),
            ) => Err(KernelSessionError::new(
                "protocol.invalidNegotiation",
                "v2 session is missing negotiated aggregate limits",
            )),
            (
                Some(NegotiatedSession {
                    protocol: ProtocolVersion::V3,
                    aggregate_limits: None,
                    ..
                }),
                KernelSessionRequest::V3(_),
            ) => Err(KernelSessionError::new(
                "protocol.invalidNegotiation",
                "v3 session is missing negotiated aggregate limits",
            )),
            (
                Some(_),
                KernelSessionRequest::V0(_)
                | KernelSessionRequest::V1(_)
                | KernelSessionRequest::V2(_)
                | KernelSessionRequest::V3(_),
            ) => Err(KernelSessionError::new(
                "protocol.mismatch",
                "request envelope does not use the negotiated protocol",
            )),
        }
    }

    /// Decodes one post-bootstrap v1 request frame and encodes its messages.
    ///
    /// Every response is encoded with
    /// [`kernel_v1::encode_response_for_request`], which enforces correlation,
    /// result type, selected range, and the request's `maxElements` bound.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] before v1 negotiation, for malformed or
    /// mixed-version frames, or for response/event encoding failures.
    pub fn handle_v1_frame(&mut self, input: &str) -> Result<Vec<String>, KernelSessionError> {
        let limits = self.v1_limits()?;
        let request = kernel_v1::decode_request(input, &limits)
            .map_err(|error| KernelSessionError::from_v1_codec(&error))?;
        let messages = self.handle(KernelSessionRequest::V1(&request))?;
        encode_v1_session_messages(&messages, &request, &limits)
    }

    /// Decodes one post-bootstrap v2 request frame and request-aware encodes
    /// every response plus event.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] before v2 negotiation, for malformed or
    /// mixed-version frames, or for response/event encoding failures.
    pub fn handle_v2_frame(&mut self, input: &str) -> Result<Vec<String>, KernelSessionError> {
        let limits = self.v2_limits()?;
        let request = kernel_v2::decode_request(input, &limits)
            .map_err(|error| KernelSessionError::from_v2_codec(&error))?;
        let messages = self.handle(KernelSessionRequest::V2(&request))?;
        encode_v2_session_messages(&messages, &request, &limits)
    }

    /// Decodes one post-bootstrap v3 request and request-aware encodes its
    /// response and events.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] before v3 negotiation, for malformed or
    /// mixed-version frames, or for response/event encoding failures.
    pub fn handle_v3_frame(&mut self, input: &str) -> Result<Vec<String>, KernelSessionError> {
        let limits = self.v3_limits()?;
        let request = kernel_v3::decode_request(input, &limits)
            .map_err(|error| KernelSessionError::from_v3_codec(&error))?;
        let messages = self.handle(KernelSessionRequest::V3(&request))?;
        encode_v3_session_messages(&messages, &request, &limits)
    }

    fn v1_limits(&self) -> Result<PreviewLimits, KernelSessionError> {
        match self.negotiated {
            Some(NegotiatedSession {
                protocol: ProtocolVersion::V1,
                preview_limits: Some(limits),
                ..
            }) => Ok(limits),
            Some(_) => Err(KernelSessionError::new(
                "protocol.mismatch",
                "the negotiated session does not accept v1 frames",
            )),
            None => Err(KernelSessionError::new(
                "protocol.bootstrapRequired",
                "initialize must succeed before post-bootstrap v1 frames",
            )),
        }
    }

    fn v2_limits(&self) -> Result<AggregateLimits, KernelSessionError> {
        match self.negotiated {
            Some(NegotiatedSession {
                protocol: ProtocolVersion::V2,
                aggregate_limits: Some(limits),
                ..
            }) => Ok(limits),
            Some(_) => Err(KernelSessionError::new(
                "protocol.mismatch",
                "the negotiated session does not accept v2 frames",
            )),
            None => Err(KernelSessionError::new(
                "protocol.bootstrapRequired",
                "initialize must succeed before post-bootstrap v2 frames",
            )),
        }
    }

    fn v3_limits(&self) -> Result<AggregateLimits, KernelSessionError> {
        match self.negotiated {
            Some(NegotiatedSession {
                protocol: ProtocolVersion::V3,
                aggregate_limits: Some(limits),
                ..
            }) => Ok(limits),
            Some(_) => Err(KernelSessionError::new(
                "protocol.mismatch",
                "the negotiated session does not accept v3 frames",
            )),
            None => Err(KernelSessionError::new(
                "protocol.bootstrapRequired",
                "initialize must succeed before post-bootstrap v3 frames",
            )),
        }
    }

    fn handle_bootstrap(
        &mut self,
        request: &kernel_v2::BootstrapRequestEnvelope,
    ) -> Result<Vec<KernelSessionMessage>, KernelSessionError> {
        if let Err(error) = request.validate() {
            return Ok(self.bootstrap_failure(
                request,
                ProtocolError::new(error.category(), error.to_string()),
            ));
        }
        if request.session_id != self.kernel.control.session_id {
            return Ok(self.bootstrap_failure(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            ));
        }

        let kernel_v2::BootstrapRequest::Initialize(initialize) = &request.request;
        let negotiated = match kernel_v2::negotiate_initialize(
            initialize,
            &self.kernel.engine.v2_capabilities(),
        ) {
            Ok(negotiated) => negotiated,
            Err(error) => {
                return Ok(self.bootstrap_failure(request, error.to_protocol_error()));
            }
        };
        let result =
            kernel_v2::initialize_result(negotiated.clone(), self.kernel.engine.implementation());
        let preview_limits = match negotiated.protocol {
            ProtocolVersion::V0 => None,
            ProtocolVersion::V1 | ProtocolVersion::V2 | ProtocolVersion::V3 => {
                Some(v1_limits_from_v2_capabilities(&result.capabilities)?)
            }
        };
        let aggregate_limits = match negotiated.protocol {
            ProtocolVersion::V2 | ProtocolVersion::V3 => Some(
                result
                    .capabilities
                    .aggregate_limits()
                    .map_err(|error| KernelSessionError::from_validation(&error))?,
            ),
            ProtocolVersion::V0 | ProtocolVersion::V1 => None,
        };
        let response = kernel_v2::BootstrapResponseEnvelope::success(
            request,
            self.kernel.control.next_id(),
            result,
        );
        response
            .validate()
            .map_err(|error| KernelSessionError::from_validation(&error))?;

        self.kernel.initialized = true;
        self.kernel.set_status(KernelStatus::Idle);
        self.negotiated = Some(NegotiatedSession {
            protocol: negotiated.protocol,
            preview_limits,
            aggregate_limits,
        });
        let idle = match negotiated.protocol {
            ProtocolVersion::V0 => {
                KernelSessionMessage::V0(self.kernel.status_message(KernelStatus::Idle))
            }
            ProtocolVersion::V1 => KernelSessionMessage::V1(kernel_v1::ServerMessage::Event(
                self.kernel.control.status_event_v1(KernelStatus::Idle),
            )),
            ProtocolVersion::V2 => KernelSessionMessage::V2(kernel_v2::ServerMessage::Event(
                self.kernel.control.status_event_v2(KernelStatus::Idle),
            )),
            ProtocolVersion::V3 => KernelSessionMessage::V3(kernel_v3::ServerMessage::Event(
                self.kernel.control.status_event_v3(KernelStatus::Idle),
            )),
        };
        Ok(vec![KernelSessionMessage::Bootstrap(response), idle])
    }

    fn bootstrap_failure(
        &self,
        request: &kernel_v2::BootstrapRequestEnvelope,
        error: ProtocolError,
    ) -> Vec<KernelSessionMessage> {
        vec![KernelSessionMessage::Bootstrap(
            kernel_v2::BootstrapResponseEnvelope::failure(
                request,
                self.kernel.control.next_id(),
                error,
            ),
        )]
    }
}

impl<E: ExecutionEngine> Kernel<E> {
    fn handle_v1_request(
        &mut self,
        request: &kernel_v1::RequestEnvelope,
        limits: &PreviewLimits,
    ) -> Vec<kernel_v1::ServerMessage> {
        if let Err(error) = request.validate(limits) {
            return vec![self.failure_v1(
                request,
                ProtocolError::new("protocol.validation", error.to_string()),
            )];
        }
        if request.session_id != self.control.session_id {
            return vec![self.failure_v1(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            )];
        }
        if self.status() == KernelStatus::Dead {
            return vec![self.failure_v1(
                request,
                ProtocolError::new("kernel.dead", "kernel has already shut down"),
            )];
        }

        match &request.request {
            kernel_v1::Request::Interrupt(_) => self.control.handle_interrupt_v1(request, limits),
            kernel_v1::Request::Execute(parameters) => self.execute_v1(request, parameters),
            kernel_v1::Request::Inspect(parameters) => self.inspect_v1(request, parameters, limits),
            kernel_v1::Request::ListWorkspace(_) => self.list_workspace_v1(request),
            kernel_v1::Request::Shutdown(_) => self.shutdown_v1(request),
        }
    }

    fn execute_v1(
        &mut self,
        request: &kernel_v1::RequestEnvelope,
        parameters: &openmat_protocol::ExecuteRequest,
    ) -> Vec<kernel_v1::ServerMessage> {
        if let Some(failure) = self.require_initialized_v1(request) {
            return vec![failure];
        }
        self.control.cancellation.reset();
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v1(KernelStatus::Busy)];
        let execution = self.engine.execute(parameters, &self.control.cancellation);
        match execution {
            Ok(ExecutionOutput { result, events }) => {
                for event in events {
                    messages.push(self.event_message_v1(event));
                }
                let interrupted = result.interrupted
                    || self.control.cancellation.is_cancelled()
                    || self.status() == KernelStatus::Interrupted;
                if interrupted && self.status() != KernelStatus::Interrupted {
                    self.set_status(KernelStatus::Interrupted);
                    messages.push(self.status_message_v1(KernelStatus::Interrupted));
                }
                messages.push(self.success_v1(
                    request,
                    kernel_v1::ResponseResult::Execute(openmat_protocol::ExecuteResult {
                        interrupted,
                    }),
                ));
            }
            Err(mut error) => {
                for event in std::mem::take(&mut error.events) {
                    messages.push(self.event_message_v1(event));
                }
                messages.push(self.failure_v1(request, error.into_protocol_error()));
            }
        }
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v1(KernelStatus::Idle));
        messages
    }

    fn inspect_v1(
        &mut self,
        request: &kernel_v1::RequestEnvelope,
        parameters: &kernel_v1::InspectRequest,
        limits: &PreviewLimits,
    ) -> Vec<kernel_v1::ServerMessage> {
        if let Some(failure) = self.require_initialized_v1(request) {
            return vec![failure];
        }
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v1(KernelStatus::Busy)];
        let response = match self.engine.inspect_v1(parameters, limits) {
            Ok(preview) => {
                let response = kernel_v1::ResponseEnvelope::success(
                    request,
                    self.control.next_id(),
                    kernel_v1::ResponseResult::Inspect(preview),
                );
                match response.validate_for_request(request, limits) {
                    Ok(()) => kernel_v1::ServerMessage::Response(response),
                    Err(error) => self.failure_v1(
                        request,
                        ProtocolError::new("engine.invalidPreview", error.to_string()),
                    ),
                }
            }
            Err(error) => self.failure_v1(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v1(KernelStatus::Idle));
        messages
    }

    fn list_workspace_v1(
        &mut self,
        request: &kernel_v1::RequestEnvelope,
    ) -> Vec<kernel_v1::ServerMessage> {
        if let Some(failure) = self.require_initialized_v1(request) {
            return vec![failure];
        }
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v1(KernelStatus::Busy)];
        let response = match self.engine.list_workspace() {
            Ok(mut variables) => {
                variables.sort_by(|left, right| left.name.cmp(&right.name));
                self.success_v1(
                    request,
                    kernel_v1::ResponseResult::ListWorkspace(WorkspaceSummary { variables }),
                )
            }
            Err(error) => self.failure_v1(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v1(KernelStatus::Idle));
        messages
    }

    fn shutdown_v1(
        &mut self,
        request: &kernel_v1::RequestEnvelope,
    ) -> Vec<kernel_v1::ServerMessage> {
        let response = match self.engine.shutdown() {
            Ok(()) => self.success_v1(
                request,
                kernel_v1::ResponseResult::Shutdown(ShutdownResult {}),
            ),
            Err(error) => self.failure_v1(request, error.into_protocol_error()),
        };
        self.set_status(KernelStatus::Dead);
        vec![response, self.status_message_v1(KernelStatus::Dead)]
    }

    fn require_initialized_v1(
        &self,
        request: &kernel_v1::RequestEnvelope,
    ) -> Option<kernel_v1::ServerMessage> {
        (!self.initialized).then(|| {
            self.failure_v1(
                request,
                ProtocolError::new(
                    "kernel.notInitialized",
                    "initialize must succeed before workspace requests",
                ),
            )
        })
    }

    fn success_v1(
        &self,
        request: &kernel_v1::RequestEnvelope,
        result: kernel_v1::ResponseResult,
    ) -> kernel_v1::ServerMessage {
        kernel_v1::ServerMessage::Response(kernel_v1::ResponseEnvelope::success(
            request,
            self.control.next_id(),
            result,
        ))
    }

    fn failure_v1(
        &self,
        request: &kernel_v1::RequestEnvelope,
        error: ProtocolError,
    ) -> kernel_v1::ServerMessage {
        kernel_v1::ServerMessage::Response(kernel_v1::ResponseEnvelope::failure(
            request,
            self.control.next_id(),
            error,
        ))
    }

    fn event_message_v1(&self, event: Event) -> kernel_v1::ServerMessage {
        kernel_v1::ServerMessage::Event(kernel_v1::EventEnvelope::new(
            &self.control.session_id,
            self.control.next_id(),
            event,
        ))
    }

    fn status_message_v1(&self, status: KernelStatus) -> kernel_v1::ServerMessage {
        kernel_v1::ServerMessage::Event(self.control.status_event_v1(status))
    }
}

impl<E: ExecutionEngine> Kernel<E> {
    fn handle_v2_request(
        &mut self,
        request: &kernel_v2::RequestEnvelope,
        limits: &AggregateLimits,
    ) -> Vec<kernel_v2::ServerMessage> {
        if let Err(error) = request.validate(limits) {
            return vec![self.failure_v2(
                request,
                ProtocolError::new("protocol.validation", error.to_string()),
            )];
        }
        if request.session_id != self.control.session_id {
            return vec![self.failure_v2(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            )];
        }
        if self.status() == KernelStatus::Dead {
            return vec![self.failure_v2(
                request,
                ProtocolError::new("kernel.dead", "kernel has already shut down"),
            )];
        }

        match &request.request {
            kernel_v2::Request::Interrupt(_) => self.control.handle_interrupt_v2(request, limits),
            kernel_v2::Request::Execute(parameters) => self.execute_v2(request, parameters),
            kernel_v2::Request::Inspect(parameters) => self.inspect_v2(request, parameters, limits),
            kernel_v2::Request::ListWorkspace(_) => self.list_workspace_v2(request),
            kernel_v2::Request::Shutdown(_) => self.shutdown_v2(request),
        }
    }

    fn execute_v2(
        &mut self,
        request: &kernel_v2::RequestEnvelope,
        parameters: &openmat_protocol::ExecuteRequest,
    ) -> Vec<kernel_v2::ServerMessage> {
        if let Some(failure) = self.require_initialized_v2(request) {
            return vec![failure];
        }
        self.control.cancellation.reset();
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v2(KernelStatus::Busy)];
        let execution = self.engine.execute(parameters, &self.control.cancellation);
        match execution {
            Ok(ExecutionOutput { result, events }) => {
                for event in events {
                    messages.push(self.event_message_v2(event));
                }
                let interrupted = result.interrupted
                    || self.control.cancellation.is_cancelled()
                    || self.status() == KernelStatus::Interrupted;
                if interrupted && self.status() != KernelStatus::Interrupted {
                    self.set_status(KernelStatus::Interrupted);
                    messages.push(self.status_message_v2(KernelStatus::Interrupted));
                }
                messages.push(self.success_v2(
                    request,
                    kernel_v2::ResponseResult::Execute(openmat_protocol::ExecuteResult {
                        interrupted,
                    }),
                ));
            }
            Err(mut error) => {
                for event in std::mem::take(&mut error.events) {
                    messages.push(self.event_message_v2(event));
                }
                messages.push(self.failure_v2(request, error.into_protocol_error()));
            }
        }
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v2(KernelStatus::Idle));
        messages
    }

    fn inspect_v2(
        &mut self,
        request: &kernel_v2::RequestEnvelope,
        parameters: &kernel_v2::InspectRequest,
        limits: &AggregateLimits,
    ) -> Vec<kernel_v2::ServerMessage> {
        if let Some(failure) = self.require_initialized_v2(request) {
            return vec![failure];
        }
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v2(KernelStatus::Busy)];
        let response = match self.engine.inspect_v2(parameters, limits) {
            Ok(preview) => {
                let response = kernel_v2::ResponseEnvelope::success(
                    request,
                    self.control.next_id(),
                    kernel_v2::ResponseResult::Inspect(preview),
                );
                match fit_v2_inspect_response(response, request, limits) {
                    Ok(response) => kernel_v2::ServerMessage::Response(response),
                    Err(error) => self.failure_v2(request, error),
                }
            }
            Err(error) => self.failure_v2(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v2(KernelStatus::Idle));
        messages
    }

    fn list_workspace_v2(
        &mut self,
        request: &kernel_v2::RequestEnvelope,
    ) -> Vec<kernel_v2::ServerMessage> {
        if let Some(failure) = self.require_initialized_v2(request) {
            return vec![failure];
        }
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v2(KernelStatus::Busy)];
        let response = match self.engine.list_workspace() {
            Ok(mut variables) => {
                variables.sort_by(|left, right| left.name.cmp(&right.name));
                self.success_v2(
                    request,
                    kernel_v2::ResponseResult::ListWorkspace(WorkspaceSummary { variables }),
                )
            }
            Err(error) => self.failure_v2(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v2(KernelStatus::Idle));
        messages
    }

    fn shutdown_v2(
        &mut self,
        request: &kernel_v2::RequestEnvelope,
    ) -> Vec<kernel_v2::ServerMessage> {
        let response = match self.engine.shutdown() {
            Ok(()) => self.success_v2(
                request,
                kernel_v2::ResponseResult::Shutdown(ShutdownResult {}),
            ),
            Err(error) => self.failure_v2(request, error.into_protocol_error()),
        };
        self.set_status(KernelStatus::Dead);
        vec![response, self.status_message_v2(KernelStatus::Dead)]
    }

    fn require_initialized_v2(
        &self,
        request: &kernel_v2::RequestEnvelope,
    ) -> Option<kernel_v2::ServerMessage> {
        (!self.initialized).then(|| {
            self.failure_v2(
                request,
                ProtocolError::new(
                    "kernel.notInitialized",
                    "initialize must succeed before workspace requests",
                ),
            )
        })
    }

    fn success_v2(
        &self,
        request: &kernel_v2::RequestEnvelope,
        result: kernel_v2::ResponseResult,
    ) -> kernel_v2::ServerMessage {
        kernel_v2::ServerMessage::Response(kernel_v2::ResponseEnvelope::success(
            request,
            self.control.next_id(),
            result,
        ))
    }

    fn failure_v2(
        &self,
        request: &kernel_v2::RequestEnvelope,
        error: ProtocolError,
    ) -> kernel_v2::ServerMessage {
        kernel_v2::ServerMessage::Response(kernel_v2::ResponseEnvelope::failure(
            request,
            self.control.next_id(),
            error,
        ))
    }

    fn event_message_v2(&self, event: Event) -> kernel_v2::ServerMessage {
        kernel_v2::ServerMessage::Event(kernel_v2::EventEnvelope::new(
            &self.control.session_id,
            self.control.next_id(),
            event,
        ))
    }

    fn status_message_v2(&self, status: KernelStatus) -> kernel_v2::ServerMessage {
        kernel_v2::ServerMessage::Event(self.control.status_event_v2(status))
    }
}

impl<E: ExecutionEngine> Kernel<E> {
    fn handle_v3_request(
        &mut self,
        request: &kernel_v3::RequestEnvelope,
        limits: &AggregateLimits,
    ) -> Vec<kernel_v3::ServerMessage> {
        if let Err(error) = request.validate(limits) {
            return vec![self.failure_v3(
                request,
                ProtocolError::new("protocol.validation", error.to_string()),
            )];
        }
        if request.session_id != self.control.session_id {
            return vec![self.failure_v3(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            )];
        }
        if self.status() == KernelStatus::Dead {
            return vec![self.failure_v3(
                request,
                ProtocolError::new("kernel.dead", "kernel has already shut down"),
            )];
        }

        match &request.request {
            kernel_v3::Request::Interrupt(_) => self.control.handle_interrupt_v3(request, limits),
            kernel_v3::Request::Execute(parameters) => self.execute_v3(request, parameters),
            kernel_v3::Request::Inspect(parameters) => self.inspect_v3(request, parameters, limits),
            kernel_v3::Request::ListWorkspace(_) => self.list_workspace_v3(request),
            kernel_v3::Request::SetVariableElement(parameters) => {
                self.set_variable_element_v3(request, parameters)
            }
            kernel_v3::Request::Shutdown(_) => self.shutdown_v3(request),
        }
    }

    fn execute_v3(
        &mut self,
        request: &kernel_v3::RequestEnvelope,
        parameters: &openmat_protocol::ExecuteRequest,
    ) -> Vec<kernel_v3::ServerMessage> {
        if let Some(failure) = self.require_initialized_v3(request) {
            return vec![failure];
        }
        self.control.cancellation.reset();
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v3(KernelStatus::Busy)];
        let execution = self.engine.execute(parameters, &self.control.cancellation);
        self.advance_workspace_revision();
        match execution {
            Ok(ExecutionOutput { result, events }) => {
                for event in events {
                    messages.push(self.event_message_v3(event));
                }
                let interrupted = result.interrupted
                    || self.control.cancellation.is_cancelled()
                    || self.status() == KernelStatus::Interrupted;
                if interrupted && self.status() != KernelStatus::Interrupted {
                    self.set_status(KernelStatus::Interrupted);
                    messages.push(self.status_message_v3(KernelStatus::Interrupted));
                }
                messages.push(self.success_v3(
                    request,
                    kernel_v3::ResponseResult::Execute(openmat_protocol::ExecuteResult {
                        interrupted,
                    }),
                ));
            }
            Err(mut error) => {
                for event in std::mem::take(&mut error.events) {
                    messages.push(self.event_message_v3(event));
                }
                messages.push(self.failure_v3(request, error.into_protocol_error()));
            }
        }
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v3(KernelStatus::Idle));
        messages
    }

    fn inspect_v3(
        &mut self,
        request: &kernel_v3::RequestEnvelope,
        parameters: &kernel_v2::InspectRequest,
        limits: &AggregateLimits,
    ) -> Vec<kernel_v3::ServerMessage> {
        if let Some(failure) = self.require_initialized_v3(request) {
            return vec![failure];
        }
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v3(KernelStatus::Busy)];
        let response = match self.engine.inspect_v2(parameters, limits) {
            Ok(preview) => self.success_v3(
                request,
                kernel_v3::ResponseResult::Inspect(kernel_v3::VersionedInspectPreview {
                    revision: self.workspace_revision,
                    preview,
                }),
            ),
            Err(error) => self.failure_v3(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v3(KernelStatus::Idle));
        messages
    }

    fn list_workspace_v3(
        &mut self,
        request: &kernel_v3::RequestEnvelope,
    ) -> Vec<kernel_v3::ServerMessage> {
        if let Some(failure) = self.require_initialized_v3(request) {
            return vec![failure];
        }
        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v3(KernelStatus::Busy)];
        let response = match self.engine.list_workspace() {
            Ok(mut variables) => {
                variables.sort_by(|left, right| left.name.cmp(&right.name));
                self.success_v3(
                    request,
                    kernel_v3::ResponseResult::ListWorkspace(
                        kernel_v3::VersionedWorkspaceSummary {
                            revision: self.workspace_revision,
                            variables,
                        },
                    ),
                )
            }
            Err(error) => self.failure_v3(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v3(KernelStatus::Idle));
        messages
    }

    fn set_variable_element_v3(
        &mut self,
        request: &kernel_v3::RequestEnvelope,
        parameters: &kernel_v3::SetVariableElementRequest,
    ) -> Vec<kernel_v3::ServerMessage> {
        if let Some(failure) = self.require_initialized_v3(request) {
            return vec![failure];
        }
        if parameters.expected_revision != self.workspace_revision {
            return vec![self.failure_v3(
                request,
                ProtocolError::new(
                    "workspace.revisionConflict",
                    format!(
                        "workspace revision is {}; edit expected {}",
                        self.workspace_revision, parameters.expected_revision,
                    ),
                ),
            )];
        }
        if self.workspace_revision >= openmat_protocol::kernel_v1::MAX_SAFE_JSON_INTEGER {
            return vec![self.failure_v3(
                request,
                ProtocolError::new(
                    "workspace.revisionExhausted",
                    "workspace revision counter cannot advance safely",
                ),
            )];
        }

        self.set_status(KernelStatus::Busy);
        let mut messages = vec![self.status_message_v3(KernelStatus::Busy)];
        let response = match self.engine.set_variable_element(parameters) {
            Ok(variable) => {
                self.advance_workspace_revision();
                self.success_v3(
                    request,
                    kernel_v3::ResponseResult::SetVariableElement(
                        kernel_v3::SetVariableElementResult {
                            revision: self.workspace_revision,
                            variable,
                        },
                    ),
                )
            }
            Err(error) => self.failure_v3(request, error.into_protocol_error()),
        };
        messages.push(response);
        self.set_status(KernelStatus::Idle);
        messages.push(self.status_message_v3(KernelStatus::Idle));
        messages
    }

    fn shutdown_v3(
        &mut self,
        request: &kernel_v3::RequestEnvelope,
    ) -> Vec<kernel_v3::ServerMessage> {
        let response = match self.engine.shutdown() {
            Ok(()) => self.success_v3(
                request,
                kernel_v3::ResponseResult::Shutdown(ShutdownResult {}),
            ),
            Err(error) => self.failure_v3(request, error.into_protocol_error()),
        };
        self.set_status(KernelStatus::Dead);
        vec![response, self.status_message_v3(KernelStatus::Dead)]
    }

    fn require_initialized_v3(
        &self,
        request: &kernel_v3::RequestEnvelope,
    ) -> Option<kernel_v3::ServerMessage> {
        (!self.initialized).then(|| {
            self.failure_v3(
                request,
                ProtocolError::new(
                    "kernel.notInitialized",
                    "initialize must succeed before workspace requests",
                ),
            )
        })
    }

    fn success_v3(
        &self,
        request: &kernel_v3::RequestEnvelope,
        result: kernel_v3::ResponseResult,
    ) -> kernel_v3::ServerMessage {
        kernel_v3::ServerMessage::Response(kernel_v3::ResponseEnvelope::success(
            request,
            self.control.next_id(),
            result,
        ))
    }

    fn failure_v3(
        &self,
        request: &kernel_v3::RequestEnvelope,
        error: ProtocolError,
    ) -> kernel_v3::ServerMessage {
        kernel_v3::ServerMessage::Response(kernel_v3::ResponseEnvelope::failure(
            request,
            self.control.next_id(),
            error,
        ))
    }

    fn event_message_v3(&self, event: Event) -> kernel_v3::ServerMessage {
        kernel_v3::ServerMessage::Event(kernel_v3::EventEnvelope::new(
            &self.control.session_id,
            self.control.next_id(),
            event,
        ))
    }

    fn status_message_v3(&self, status: KernelStatus) -> kernel_v3::ServerMessage {
        kernel_v3::ServerMessage::Event(self.control.status_event_v3(status))
    }

    fn advance_workspace_revision(&mut self) {
        self.workspace_revision = self
            .workspace_revision
            .saturating_add(1)
            .min(openmat_protocol::kernel_v1::MAX_SAFE_JSON_INTEGER);
    }
}

impl KernelControl {
    fn handle_interrupt_v1(
        &self,
        request: &kernel_v1::RequestEnvelope,
        limits: &PreviewLimits,
    ) -> Vec<kernel_v1::ServerMessage> {
        if let Err(error) = request.validate(limits) {
            return vec![self.failure_v1(
                request,
                ProtocolError::new("protocol.validation", error.to_string()),
            )];
        }
        if request.session_id != self.session_id {
            return vec![self.failure_v1(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            )];
        }
        if !matches!(request.request, kernel_v1::Request::Interrupt(_)) {
            return vec![self.failure_v1(
                request,
                ProtocolError::new(
                    "protocol.controlRequest",
                    "control path accepts only interrupt requests",
                ),
            )];
        }

        let status = self.status();
        let accepted = matches!(status, KernelStatus::Busy | KernelStatus::Interrupted);
        let response = kernel_v1::ServerMessage::Response(kernel_v1::ResponseEnvelope::success(
            request,
            self.next_id(),
            kernel_v1::ResponseResult::Interrupt(openmat_protocol::InterruptResult { accepted }),
        ));
        if !accepted {
            return vec![response];
        }

        self.cancellation.cancel();
        if status == KernelStatus::Busy {
            self.state.store(
                super::status_to_u8(KernelStatus::Interrupted),
                std::sync::atomic::Ordering::Release,
            );
            vec![
                response,
                kernel_v1::ServerMessage::Event(self.status_event_v1(KernelStatus::Interrupted)),
            ]
        } else {
            vec![response]
        }
    }

    fn status_event_v1(&self, status: KernelStatus) -> kernel_v1::EventEnvelope {
        kernel_v1::EventEnvelope::new(
            &self.session_id,
            self.next_id(),
            Event::Status(openmat_protocol::StatusEvent { status }),
        )
    }

    fn failure_v1(
        &self,
        request: &kernel_v1::RequestEnvelope,
        error: ProtocolError,
    ) -> kernel_v1::ServerMessage {
        kernel_v1::ServerMessage::Response(kernel_v1::ResponseEnvelope::failure(
            request,
            self.next_id(),
            error,
        ))
    }

    fn handle_interrupt_v2(
        &self,
        request: &kernel_v2::RequestEnvelope,
        limits: &AggregateLimits,
    ) -> Vec<kernel_v2::ServerMessage> {
        if let Err(error) = request.validate(limits) {
            return vec![self.failure_v2(
                request,
                ProtocolError::new("protocol.validation", error.to_string()),
            )];
        }
        if request.session_id != self.session_id {
            return vec![self.failure_v2(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            )];
        }
        if !matches!(request.request, kernel_v2::Request::Interrupt(_)) {
            return vec![self.failure_v2(
                request,
                ProtocolError::new(
                    "protocol.controlRequest",
                    "control path accepts only interrupt requests",
                ),
            )];
        }

        let status = self.status();
        let accepted = matches!(status, KernelStatus::Busy | KernelStatus::Interrupted);
        let response = kernel_v2::ServerMessage::Response(kernel_v2::ResponseEnvelope::success(
            request,
            self.next_id(),
            kernel_v2::ResponseResult::Interrupt(openmat_protocol::InterruptResult { accepted }),
        ));
        if !accepted {
            return vec![response];
        }

        self.cancellation.cancel();
        if status == KernelStatus::Busy {
            self.state.store(
                super::status_to_u8(KernelStatus::Interrupted),
                std::sync::atomic::Ordering::Release,
            );
            vec![
                response,
                kernel_v2::ServerMessage::Event(self.status_event_v2(KernelStatus::Interrupted)),
            ]
        } else {
            vec![response]
        }
    }

    fn status_event_v2(&self, status: KernelStatus) -> kernel_v2::EventEnvelope {
        kernel_v2::EventEnvelope::new(
            &self.session_id,
            self.next_id(),
            Event::Status(openmat_protocol::StatusEvent { status }),
        )
    }

    fn failure_v2(
        &self,
        request: &kernel_v2::RequestEnvelope,
        error: ProtocolError,
    ) -> kernel_v2::ServerMessage {
        kernel_v2::ServerMessage::Response(kernel_v2::ResponseEnvelope::failure(
            request,
            self.next_id(),
            error,
        ))
    }

    fn handle_interrupt_v3(
        &self,
        request: &kernel_v3::RequestEnvelope,
        limits: &AggregateLimits,
    ) -> Vec<kernel_v3::ServerMessage> {
        if let Err(error) = request.validate(limits) {
            return vec![self.failure_v3(
                request,
                ProtocolError::new("protocol.validation", error.to_string()),
            )];
        }
        if request.session_id != self.session_id {
            return vec![self.failure_v3(
                request,
                ProtocolError::new(
                    "protocol.sessionMismatch",
                    "request session does not match this kernel session",
                ),
            )];
        }
        if !matches!(request.request, kernel_v3::Request::Interrupt(_)) {
            return vec![self.failure_v3(
                request,
                ProtocolError::new(
                    "protocol.controlRequest",
                    "control path accepts only interrupt requests",
                ),
            )];
        }

        let status = self.status();
        let accepted = matches!(status, KernelStatus::Busy | KernelStatus::Interrupted);
        let response = kernel_v3::ServerMessage::Response(kernel_v3::ResponseEnvelope::success(
            request,
            self.next_id(),
            kernel_v3::ResponseResult::Interrupt(openmat_protocol::InterruptResult { accepted }),
        ));
        if !accepted {
            return vec![response];
        }

        self.cancellation.cancel();
        if status == KernelStatus::Busy {
            self.state.store(
                super::status_to_u8(KernelStatus::Interrupted),
                std::sync::atomic::Ordering::Release,
            );
            vec![
                response,
                kernel_v3::ServerMessage::Event(self.status_event_v3(KernelStatus::Interrupted)),
            ]
        } else {
            vec![response]
        }
    }

    fn status_event_v3(&self, status: KernelStatus) -> kernel_v3::EventEnvelope {
        kernel_v3::EventEnvelope::new(
            &self.session_id,
            self.next_id(),
            Event::Status(openmat_protocol::StatusEvent { status }),
        )
    }

    fn failure_v3(
        &self,
        request: &kernel_v3::RequestEnvelope,
        error: ProtocolError,
    ) -> kernel_v3::ServerMessage {
        kernel_v3::ServerMessage::Response(kernel_v3::ResponseEnvelope::failure(
            request,
            self.next_id(),
            error,
        ))
    }
}

/// Independent negotiated control path for interrupt requests.
#[derive(Clone, Debug)]
pub struct KernelSessionControl {
    control: KernelControl,
    negotiated: NegotiatedSession,
}

impl KernelSessionControl {
    /// Returns the latest lifecycle state.
    #[must_use]
    pub fn status(&self) -> KernelStatus {
        self.control.status()
    }

    /// Handles one typed control request using the locked session protocol.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] for protocol mixing or an invalid
    /// engine-produced message. Well-formed non-interrupt requests receive a
    /// structured `protocol.controlRequest` response.
    pub fn handle(
        &self,
        request: KernelSessionRequest<'_>,
    ) -> Result<Vec<KernelSessionMessage>, KernelSessionError> {
        match (self.negotiated, request) {
            (
                NegotiatedSession {
                    protocol: ProtocolVersion::V0,
                    ..
                },
                KernelSessionRequest::V0(request),
            ) => {
                validate_typed_protocol(ProtocolVersion::V0, &request.protocol)?;
                Ok(self
                    .control
                    .handle_interrupt(request)
                    .into_iter()
                    .map(KernelSessionMessage::V0)
                    .collect())
            }
            (
                NegotiatedSession {
                    protocol: ProtocolVersion::V1,
                    preview_limits: Some(limits),
                    ..
                },
                KernelSessionRequest::V1(request),
            ) => {
                validate_typed_protocol(ProtocolVersion::V1, &request.protocol)?;
                let messages = self.control.handle_interrupt_v1(request, &limits);
                validate_v1_messages(&messages, request, &limits)?;
                Ok(messages.into_iter().map(KernelSessionMessage::V1).collect())
            }
            (
                NegotiatedSession {
                    protocol: ProtocolVersion::V2,
                    aggregate_limits: Some(limits),
                    ..
                },
                KernelSessionRequest::V2(request),
            ) => {
                validate_typed_protocol(ProtocolVersion::V2, &request.protocol)?;
                let messages = self.control.handle_interrupt_v2(request, &limits);
                validate_v2_messages(&messages, request, &limits)?;
                Ok(messages.into_iter().map(KernelSessionMessage::V2).collect())
            }
            (
                NegotiatedSession {
                    protocol: ProtocolVersion::V3,
                    aggregate_limits: Some(limits),
                    ..
                },
                KernelSessionRequest::V3(request),
            ) => {
                validate_typed_protocol(ProtocolVersion::V3, &request.protocol)?;
                let messages = self.control.handle_interrupt_v3(request, &limits);
                validate_v3_messages(&messages, request, &limits)?;
                Ok(messages.into_iter().map(KernelSessionMessage::V3).collect())
            }
            (_, KernelSessionRequest::Bootstrap(_)) => Err(KernelSessionError::new(
                "protocol.controlRequest",
                "control path does not accept bootstrap initialize",
            )),
            _ => Err(KernelSessionError::new(
                "protocol.mismatch",
                "control request does not use the negotiated protocol",
            )),
        }
    }

    /// Decodes and handles one v1 interrupt frame with request-aware encoding.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] unless this is a negotiated v1 session,
    /// or when decoding, validation, or encoding fails.
    pub fn handle_v1_frame(&self, input: &str) -> Result<Vec<String>, KernelSessionError> {
        let NegotiatedSession {
            protocol: ProtocolVersion::V1,
            preview_limits: Some(limits),
            ..
        } = self.negotiated
        else {
            return Err(KernelSessionError::new(
                "protocol.mismatch",
                "the negotiated control path does not accept v1 frames",
            ));
        };
        let request = kernel_v1::decode_request(input, &limits)
            .map_err(|error| KernelSessionError::from_v1_codec(&error))?;
        let messages = self.handle(KernelSessionRequest::V1(&request))?;
        encode_v1_session_messages(&messages, &request, &limits)
    }

    /// Decodes and handles one v2 interrupt frame with request-aware encoding.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] unless this is a negotiated v2 session,
    /// or when decoding, validation, or encoding fails.
    pub fn handle_v2_frame(&self, input: &str) -> Result<Vec<String>, KernelSessionError> {
        let NegotiatedSession {
            protocol: ProtocolVersion::V2,
            aggregate_limits: Some(limits),
            ..
        } = self.negotiated
        else {
            return Err(KernelSessionError::new(
                "protocol.mismatch",
                "the negotiated control path does not accept v2 frames",
            ));
        };
        let request = kernel_v2::decode_request(input, &limits)
            .map_err(|error| KernelSessionError::from_v2_codec(&error))?;
        let messages = self.handle(KernelSessionRequest::V2(&request))?;
        encode_v2_session_messages(&messages, &request, &limits)
    }

    /// Decodes and handles one v3 interrupt frame with request-aware encoding.
    ///
    /// # Errors
    ///
    /// Returns [`KernelSessionError`] unless this is a negotiated v3 session,
    /// or when decoding, validation, or encoding fails.
    pub fn handle_v3_frame(&self, input: &str) -> Result<Vec<String>, KernelSessionError> {
        let NegotiatedSession {
            protocol: ProtocolVersion::V3,
            aggregate_limits: Some(limits),
            ..
        } = self.negotiated
        else {
            return Err(KernelSessionError::new(
                "protocol.mismatch",
                "the negotiated control path does not accept v3 frames",
            ));
        };
        let request = kernel_v3::decode_request(input, &limits)
            .map_err(|error| KernelSessionError::from_v3_codec(&error))?;
        let messages = self.handle(KernelSessionRequest::V3(&request))?;
        encode_v3_session_messages(&messages, &request, &limits)
    }
}

fn validate_typed_protocol(
    selected: ProtocolVersion,
    envelope_protocol: &str,
) -> Result<(), KernelSessionError> {
    kernel_v2::validate_session_protocol(selected, envelope_protocol)
        .map_err(|error| KernelSessionError::new(error.category(), error.to_string()))
}

fn validate_v1_messages(
    messages: &[kernel_v1::ServerMessage],
    request: &kernel_v1::RequestEnvelope,
    limits: &PreviewLimits,
) -> Result<(), KernelSessionError> {
    for message in messages {
        match message {
            kernel_v1::ServerMessage::Response(response) => response
                .validate_for_request(request, limits)
                .map_err(|error| KernelSessionError::from_validation(&error))?,
            kernel_v1::ServerMessage::Event(event) => event
                .validate()
                .map_err(|error| KernelSessionError::from_validation(&error))?,
        }
    }
    Ok(())
}

fn validate_v2_messages(
    messages: &[kernel_v2::ServerMessage],
    request: &kernel_v2::RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<(), KernelSessionError> {
    for message in messages {
        match message {
            kernel_v2::ServerMessage::Response(response) => response
                .validate_for_request(request, limits)
                .map_err(|error| KernelSessionError::from_validation(&error))?,
            kernel_v2::ServerMessage::Event(event) => event
                .validate()
                .map_err(|error| KernelSessionError::from_validation(&error))?,
        }
    }
    Ok(())
}

fn validate_v3_messages(
    messages: &[kernel_v3::ServerMessage],
    request: &kernel_v3::RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<(), KernelSessionError> {
    for message in messages {
        match message {
            kernel_v3::ServerMessage::Response(response) => response
                .validate_for_request(request, limits)
                .map_err(|error| KernelSessionError::from_validation(&error))?,
            kernel_v3::ServerMessage::Event(event) => event
                .validate()
                .map_err(|error| KernelSessionError::from_validation(&error))?,
        }
    }
    Ok(())
}

fn encode_v1_session_messages(
    messages: &[KernelSessionMessage],
    request: &kernel_v1::RequestEnvelope,
    limits: &PreviewLimits,
) -> Result<Vec<String>, KernelSessionError> {
    let mut encoded = Vec::with_capacity(messages.len());
    for message in messages {
        let frame = match message {
            KernelSessionMessage::V1(kernel_v1::ServerMessage::Response(response)) => {
                kernel_v1::encode_response_for_request(response, request, limits)
                    .map_err(|error| KernelSessionError::from_v1_codec(&error))?
            }
            KernelSessionMessage::V1(kernel_v1::ServerMessage::Event(event)) => {
                kernel_v1::encode_event(event)
                    .map_err(|error| KernelSessionError::from_v1_codec(&error))?
            }
            KernelSessionMessage::Bootstrap(_)
            | KernelSessionMessage::V0(_)
            | KernelSessionMessage::V2(_)
            | KernelSessionMessage::V3(_) => {
                return Err(KernelSessionError::new(
                    "protocol.invalidSessionState",
                    "v1 frame handler produced a non-v1 message",
                ));
            }
        };
        encoded.push(frame);
    }
    Ok(encoded)
}

fn encode_v2_session_messages(
    messages: &[KernelSessionMessage],
    request: &kernel_v2::RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<Vec<String>, KernelSessionError> {
    let mut encoded = Vec::with_capacity(messages.len());
    for message in messages {
        let frame = match message {
            KernelSessionMessage::V2(kernel_v2::ServerMessage::Response(response)) => {
                kernel_v2::encode_response_for_request(response, request, limits)
                    .map_err(|error| KernelSessionError::from_v2_codec(&error))?
            }
            KernelSessionMessage::V2(kernel_v2::ServerMessage::Event(event)) => {
                kernel_v2::encode_event(event)
                    .map_err(|error| KernelSessionError::from_v2_codec(&error))?
            }
            KernelSessionMessage::Bootstrap(_)
            | KernelSessionMessage::V0(_)
            | KernelSessionMessage::V1(_)
            | KernelSessionMessage::V3(_) => {
                return Err(KernelSessionError::new(
                    "protocol.invalidSessionState",
                    "v2 frame handler produced a non-v2 message",
                ));
            }
        };
        encoded.push(frame);
    }
    Ok(encoded)
}

fn encode_v3_session_messages(
    messages: &[KernelSessionMessage],
    request: &kernel_v3::RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<Vec<String>, KernelSessionError> {
    let mut encoded = Vec::with_capacity(messages.len());
    for message in messages {
        let frame = match message {
            KernelSessionMessage::V3(kernel_v3::ServerMessage::Response(response)) => {
                kernel_v3::encode_response_for_request(response, request, limits)
                    .map_err(|error| KernelSessionError::from_v3_codec(&error))?
            }
            KernelSessionMessage::V3(kernel_v3::ServerMessage::Event(event)) => {
                kernel_v3::encode_event(event)
                    .map_err(|error| KernelSessionError::from_v3_codec(&error))?
            }
            KernelSessionMessage::Bootstrap(_)
            | KernelSessionMessage::V0(_)
            | KernelSessionMessage::V1(_)
            | KernelSessionMessage::V2(_) => {
                return Err(KernelSessionError::new(
                    "protocol.invalidSessionState",
                    "v3 frame handler produced a non-v3 message",
                ));
            }
        };
        encoded.push(frame);
    }
    Ok(encoded)
}

fn v1_limits_from_v2_capabilities(
    capabilities: &kernel_v2::Capabilities,
) -> Result<PreviewLimits, KernelSessionError> {
    let string_limit = capabilities.max_string_element_code_units.ok_or_else(|| {
        KernelSessionError::new(
            "protocol.invalidNegotiation",
            "v1 or v2 selection is missing maxStringElementCodeUnits",
        )
    })?;
    let aggregate_limit = capabilities.max_preview_code_units.ok_or_else(|| {
        KernelSessionError::new(
            "protocol.invalidNegotiation",
            "v1 or v2 selection is missing maxPreviewCodeUnits",
        )
    })?;
    PreviewLimits::new(
        capabilities.max_preview_elements,
        string_limit,
        aggregate_limit,
    )
    .map_err(|error| KernelSessionError::from_validation(&error))
}

fn fit_v2_inspect_response(
    mut response: kernel_v2::ResponseEnvelope,
    request: &kernel_v2::RequestEnvelope,
    limits: &AggregateLimits,
) -> Result<kernel_v2::ResponseEnvelope, ProtocolError> {
    loop {
        match kernel_v2::encode_response_for_request(&response, request, limits) {
            Ok(_) => return Ok(response),
            Err(kernel_v2::CodecError::FrameTooLarge { .. }) => {
                if !truncate_v2_inspect_response(&mut response) {
                    return Err(ProtocolError::new(
                        "workspace.previewLimit",
                        "the indivisible inspect response header exceeds the v2 frame limit",
                    ));
                }
            }
            Err(error) => {
                return Err(ProtocolError::new(
                    "engine.invalidPreview",
                    format!("engine-produced v2 inspect response is invalid: {error}"),
                ));
            }
        }
    }
}

fn truncate_v2_inspect_response(response: &mut kernel_v2::ResponseEnvelope) -> bool {
    let Some(kernel_v2::ResponseResult::Inspect(preview)) = response.result.as_mut() else {
        return false;
    };
    match preview {
        kernel_v2::InspectPreview::Matrix(preview) => {
            if preview.values.pop().is_none() {
                return false;
            }
            let Some(omitted) = preview.truncation.omitted_elements.checked_add(1) else {
                return false;
            };
            preview.truncation.omitted_elements = omitted;
            preview.truncation.truncated = true;
            true
        }
        kernel_v2::InspectPreview::Aggregate(preview) => {
            let removed = match preview {
                kernel_v2::AggregatePreview::Cell { items, .. } => items.pop().is_some(),
                kernel_v2::AggregatePreview::Struct { records, .. } => records.pop().is_some(),
                kernel_v2::AggregatePreview::Table {
                    variable_names,
                    variables,
                    ..
                } => {
                    if variables.is_empty() || variable_names.len() != variables.len() {
                        false
                    } else {
                        variables.pop();
                        variable_names.pop();
                        true
                    }
                }
            };
            if !removed {
                return false;
            }
            let truncation = match preview {
                kernel_v2::AggregatePreview::Cell { truncation, .. }
                | kernel_v2::AggregatePreview::Struct { truncation, .. }
                | kernel_v2::AggregatePreview::Table { truncation, .. } => truncation,
            };
            let Some(omitted) = truncation.omitted_elements.checked_add(1) else {
                return false;
            };
            truncation.omitted_elements = omitted;
            truncation.truncated = true;
            let Some(recomputed) = recompute_aggregate_usage(preview) else {
                return false;
            };
            match preview {
                kernel_v2::AggregatePreview::Cell { usage, .. }
                | kernel_v2::AggregatePreview::Struct { usage, .. }
                | kernel_v2::AggregatePreview::Table { usage, .. } => *usage = recomputed,
            }
            true
        }
    }
}

#[allow(clippy::too_many_lines)]
fn recompute_aggregate_usage(
    preview: &kernel_v2::AggregatePreview,
) -> Option<kernel_v2::PreviewUsage> {
    let mut usage = kernel_v2::PreviewUsage {
        nodes: 1,
        elements: 0,
        code_units: 0,
        depth: 0,
    };
    let mut stack = Vec::new();
    match preview {
        kernel_v2::AggregatePreview::Cell { items, .. } => {
            usage.elements = u64::try_from(items.len()).ok()?;
            stack.extend(items.iter().rev().map(|value| (value, 1_u64)));
        }
        kernel_v2::AggregatePreview::Struct {
            fields, records, ..
        } => {
            usage.elements = u64::try_from(records.len()).ok()?;
            for field in fields {
                usage.code_units = usage
                    .code_units
                    .checked_add(u64::try_from(field.encode_utf16().count()).ok()?)?;
            }
            for record in records.iter().rev() {
                for field in fields.iter().rev() {
                    let value = record
                        .entries
                        .iter()
                        .find_map(|(name, value)| (name == field).then_some(value))?;
                    stack.push((value, 1_u64));
                }
            }
        }
        kernel_v2::AggregatePreview::Table {
            variable_names,
            variables,
            ..
        } => {
            if variable_names.len() != variables.len() {
                return None;
            }
            usage.elements = u64::try_from(variables.len()).ok()?;
            for name in variable_names {
                usage.code_units = usage
                    .code_units
                    .checked_add(u64::try_from(name.encode_utf16().count()).ok()?)?;
            }
            stack.extend(variables.iter().rev().map(|variable| (variable, 1_u64)));
        }
    }
    while let Some((value, depth)) = stack.pop() {
        usage.nodes = usage.nodes.checked_add(1)?;
        usage.elements = usage.elements.checked_add(value.numel)?;
        usage.depth = usage.depth.max(depth);
        match &value.payload {
            kernel_v2::ExactPayload::Char { code_units } => {
                usage.code_units = usage
                    .code_units
                    .checked_add(u64::try_from(code_units.len()).ok()?)?;
            }
            kernel_v2::ExactPayload::String {
                string_code_units, ..
            } => {
                for element in string_code_units {
                    usage.code_units = usage
                        .code_units
                        .checked_add(u64::try_from(element.len()).ok()?)?;
                }
            }
            kernel_v2::ExactPayload::Cell { items } => {
                let child_depth = depth.checked_add(1)?;
                stack.extend(items.iter().rev().map(|item| (item, child_depth)));
            }
            kernel_v2::ExactPayload::Struct { fields, records } => {
                for field in fields {
                    usage.code_units = usage
                        .code_units
                        .checked_add(u64::try_from(field.encode_utf16().count()).ok()?)?;
                }
                let child_depth = depth.checked_add(1)?;
                for record in records.iter().rev() {
                    for field in fields.iter().rev() {
                        let child = record
                            .entries
                            .iter()
                            .find_map(|(name, value)| (name == field).then_some(value))?;
                        stack.push((child, child_depth));
                    }
                }
            }
            kernel_v2::ExactPayload::Table {
                variable_names,
                variables,
            } => {
                for name in variable_names {
                    usage.code_units = usage
                        .code_units
                        .checked_add(u64::try_from(name.encode_utf16().count()).ok()?)?;
                }
                let child_depth = depth.checked_add(1)?;
                stack.extend(
                    variables
                        .iter()
                        .rev()
                        .map(|variable| (variable, child_depth)),
                );
            }
            kernel_v2::ExactPayload::Numeric { .. }
            | kernel_v2::ExactPayload::Integer { .. }
            | kernel_v2::ExactPayload::Logical { .. } => {}
        }
    }
    Some(usage)
}

/// Stable session-boundary error for requests that cannot receive a wire reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelSessionError {
    category: String,
    message: String,
}

impl KernelSessionError {
    fn new(category: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            message: message.into(),
        }
    }

    fn from_v1_codec(error: &kernel_v1::CodecError) -> Self {
        let category = error
            .validation_category()
            .unwrap_or("protocol.invalidMessage");
        Self::new(category, error.to_string())
    }

    fn from_v2_codec(error: &kernel_v2::CodecError) -> Self {
        let category = error
            .validation_category()
            .unwrap_or("protocol.invalidMessage");
        Self::new(category, error.to_string())
    }

    fn from_v3_codec(error: &kernel_v3::CodecError) -> Self {
        let category = error
            .validation_category()
            .unwrap_or("protocol.invalidMessage");
        Self::new(category, error.to_string())
    }

    fn from_validation(error: &openmat_protocol::ValidationError) -> Self {
        Self::new(error.category(), error.to_string())
    }

    /// Returns the stable error category.
    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }

    /// Returns the human-readable error detail.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for KernelSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl Error for KernelSessionError {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use openmat_protocol::kernel_v1::{
        InspectRequest, MatrixPreview, MatrixRange, PreviewValue, Request,
        RequestEnvelope as V1RequestEnvelope, ResponseEnvelope as V1ResponseEnvelope,
        ResponseResult as V1ResponseResult,
    };
    use openmat_protocol::kernel_v2::{BootstrapResponseResult, Capabilities, InitializeRequest};
    use openmat_protocol::{
        Capabilities as V0Capabilities, ExecuteRequest, ExecuteResult, ExecutionMode,
        ImplementationInfo, InspectRequest as V0InspectRequest, InterruptRequest,
        MatrixPreview as V0MatrixPreview, PreviewTruncation, StreamEvent, StreamKind,
        VariableSummary,
    };

    use super::*;
    use crate::{CancellationToken, EngineError, ExecutionOutput};

    struct MockEngine {
        started: Option<Arc<AtomicBool>>,
        wait_for_interrupt: bool,
        invalid_preview: bool,
        v2_preview: Option<kernel_v2::InspectPreview>,
        value: f64,
    }

    impl MockEngine {
        fn ordinary() -> Self {
            Self {
                started: None,
                wait_for_interrupt: false,
                invalid_preview: false,
                v2_preview: None,
                value: 42.0,
            }
        }
    }

    impl ExecutionEngine for MockEngine {
        fn implementation(&self) -> ImplementationInfo {
            ImplementationInfo {
                name: "session-mock".to_owned(),
                version: "1".to_owned(),
            }
        }

        fn capabilities(&self) -> V0Capabilities {
            V0Capabilities {
                execution_modes: vec![
                    ExecutionMode::File,
                    ExecutionMode::Cell,
                    ExecutionMode::Repl,
                ],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: 64,
                interrupt: true,
                workspace_delta: true,
            }
        }

        fn v2_capabilities(&self) -> Capabilities {
            Capabilities {
                execution_modes: vec![
                    ExecutionMode::File,
                    ExecutionMode::Cell,
                    ExecutionMode::Repl,
                ],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: openmat_protocol::MAX_PREVIEW_ELEMENTS,
                max_string_element_code_units: Some(kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS),
                max_preview_code_units: Some(kernel_v1::MAX_PREVIEW_CODE_UNITS),
                max_aggregate_nodes: Some(kernel_v2::MAX_AGGREGATE_NODES),
                max_aggregate_elements: Some(kernel_v2::MAX_AGGREGATE_ELEMENTS),
                max_aggregate_depth: Some(kernel_v2::MAX_AGGREGATE_DEPTH),
                interrupt: true,
                workspace_delta: true,
            }
        }

        fn execute(
            &mut self,
            request: &ExecuteRequest,
            cancellation: &CancellationToken,
        ) -> Result<ExecutionOutput, EngineError> {
            if let Some(started) = &self.started {
                started.store(true, Ordering::Release);
            }
            while self.wait_for_interrupt && !cancellation.is_cancelled() {
                thread::yield_now();
            }
            Ok(ExecutionOutput {
                result: ExecuteResult {
                    interrupted: cancellation.is_cancelled(),
                },
                events: vec![Event::Stream(StreamEvent {
                    stream: StreamKind::Stdout,
                    text: request.code.clone(),
                })],
            })
        }

        fn inspect(&mut self, _request: &V0InspectRequest) -> Result<V0MatrixPreview, EngineError> {
            unreachable!("v0 inspect is not used in session tests")
        }

        fn inspect_v1(
            &mut self,
            request: &InspectRequest,
            _limits: &PreviewLimits,
        ) -> Result<MatrixPreview, EngineError> {
            let selected_range = if self.invalid_preview {
                MatrixRange {
                    start: vec![1, 2],
                    size: vec![1, 1],
                }
            } else {
                request.range.clone()
            };
            Ok(MatrixPreview {
                class: "double".to_owned(),
                dimensions: vec![1, 1],
                complex: false,
                selected_range,
                values: vec![PreviewValue::Number { value: self.value }],
                truncation: PreviewTruncation {
                    truncated: false,
                    omitted_elements: 0,
                },
            })
        }

        fn inspect_v2(
            &mut self,
            request: &kernel_v2::InspectRequest,
            limits: &AggregateLimits,
        ) -> Result<kernel_v2::InspectPreview, EngineError> {
            if let Some(preview) = &self.v2_preview {
                return Ok(preview.clone());
            }
            self.inspect_v1(request, &limits.preview_limits())
                .map(kernel_v2::InspectPreview::Matrix)
        }

        fn list_workspace(&mut self) -> Result<Vec<VariableSummary>, EngineError> {
            Ok(vec![VariableSummary {
                name: "answer".to_owned(),
                class: "double".to_owned(),
                dimensions: vec![1, 1],
                complex: false,
                bytes: Some(8),
            }])
        }

        fn set_variable_element(
            &mut self,
            request: &kernel_v3::SetVariableElementRequest,
        ) -> Result<VariableSummary, EngineError> {
            if request.name != "answer" || request.indices != [1, 1] {
                return Err(EngineError::new(
                    "workspace.indexOutOfBounds",
                    "mock workspace element does not exist",
                ));
            }
            let (real, imaginary) = request
                .value
                .components()
                .map_err(|error| EngineError::new("protocol.validation", error.to_string()))?;
            if imaginary != 0.0 {
                return Err(EngineError::new(
                    "workspace.editUnsupported",
                    "mock answer only accepts real values",
                ));
            }
            self.value = real;
            self.list_workspace().map(|variables| variables[0].clone())
        }

        fn shutdown(&mut self) -> Result<(), EngineError> {
            Ok(())
        }
    }

    fn client_capabilities() -> Capabilities {
        Capabilities {
            execution_modes: vec![
                ExecutionMode::File,
                ExecutionMode::Cell,
                ExecutionMode::Repl,
            ],
            display_mime_types: vec!["text/plain".to_owned()],
            max_preview_elements: 64,
            max_string_element_code_units: Some(kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS),
            max_preview_code_units: Some(kernel_v1::MAX_PREVIEW_CODE_UNITS),
            max_aggregate_nodes: None,
            max_aggregate_elements: None,
            max_aggregate_depth: None,
            interrupt: true,
            workspace_delta: true,
        }
    }

    fn bootstrap(
        protocols: Vec<String>,
        capabilities: Capabilities,
    ) -> kernel_v2::BootstrapRequestEnvelope {
        kernel_v2::BootstrapRequestEnvelope::new(
            "session-1",
            "initialize-1",
            InitializeRequest {
                client: ImplementationInfo {
                    name: "test-client".to_owned(),
                    version: "1".to_owned(),
                },
                supported_protocols: protocols,
                capabilities,
            },
        )
    }

    fn v1_bootstrap() -> kernel_v2::BootstrapRequestEnvelope {
        bootstrap(
            vec![
                kernel_v1::PROTOCOL_V1.to_owned(),
                openmat_protocol::PROTOCOL_V0.to_owned(),
            ],
            client_capabilities(),
        )
    }

    fn v2_client_capabilities() -> Capabilities {
        Capabilities {
            execution_modes: vec![
                ExecutionMode::File,
                ExecutionMode::Cell,
                ExecutionMode::Repl,
            ],
            display_mime_types: vec!["text/plain".to_owned()],
            max_preview_elements: openmat_protocol::MAX_PREVIEW_ELEMENTS,
            max_string_element_code_units: Some(kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS),
            max_preview_code_units: Some(kernel_v1::MAX_PREVIEW_CODE_UNITS),
            max_aggregate_nodes: Some(kernel_v2::MAX_AGGREGATE_NODES),
            max_aggregate_elements: Some(kernel_v2::MAX_AGGREGATE_ELEMENTS),
            max_aggregate_depth: Some(kernel_v2::MAX_AGGREGATE_DEPTH),
            interrupt: true,
            workspace_delta: true,
        }
    }

    fn v2_bootstrap() -> kernel_v2::BootstrapRequestEnvelope {
        bootstrap(
            kernel_v2::production_protocol_offer(),
            v2_client_capabilities(),
        )
    }

    fn v3_bootstrap() -> kernel_v2::BootstrapRequestEnvelope {
        bootstrap(
            vec![kernel_v3::PROTOCOL_V3.to_owned()],
            v2_client_capabilities(),
        )
    }

    fn negotiated_v2_limits() -> AggregateLimits {
        v2_client_capabilities().aggregate_limits().unwrap()
    }

    fn negotiated_limits() -> PreviewLimits {
        PreviewLimits::new(
            64,
            kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS,
            kernel_v1::MAX_PREVIEW_CODE_UNITS,
        )
        .unwrap()
    }

    fn decoded_response(
        frames: &[String],
        request: &V1RequestEnvelope,
        limits: &PreviewLimits,
    ) -> V1ResponseEnvelope {
        frames
            .iter()
            .find_map(|frame| kernel_v1::decode_response_for_request(frame, request, limits).ok())
            .expect("request-aware response frame")
    }

    fn decoded_v2_response(
        frames: &[String],
        request: &kernel_v2::RequestEnvelope,
        limits: &AggregateLimits,
    ) -> kernel_v2::ResponseEnvelope {
        frames
            .iter()
            .find_map(|frame| kernel_v2::decode_response_for_request(frame, request, limits).ok())
            .expect("request-aware v2 response frame")
    }

    fn decoded_v3_response(
        frames: &[String],
        request: &kernel_v3::RequestEnvelope,
        limits: &AggregateLimits,
    ) -> kernel_v3::ResponseEnvelope {
        frames
            .iter()
            .find_map(|frame| kernel_v3::decode_response_for_request(frame, request, limits).ok())
            .expect("request-aware v3 response frame")
    }

    fn logical_exact(value: bool) -> kernel_v2::ExactValue {
        kernel_v2::ExactValue {
            class: "logical".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            payload: kernel_v2::ExactPayload::Logical {
                logical: vec![value],
            },
        }
    }

    #[test]
    fn bootstrap_selects_v1_and_locks_every_following_request() {
        let mut session = KernelSession::new("session-1", MockEngine::ordinary());
        let early = V1RequestEnvelope::new(
            "session-1",
            "early",
            Request::ListWorkspace(openmat_protocol::ListWorkspaceRequest {}),
        );
        assert_eq!(
            session
                .handle(KernelSessionRequest::V1(&early))
                .expect_err("only bootstrap is legal before negotiation")
                .category(),
            "protocol.bootstrapRequired"
        );

        let bootstrap = v1_bootstrap();
        let messages = session
            .handle(KernelSessionRequest::Bootstrap(&bootstrap))
            .unwrap();
        let KernelSessionMessage::Bootstrap(response) = &messages[0] else {
            panic!("bootstrap response")
        };
        let Some(BootstrapResponseResult::Initialize(result)) = &response.result else {
            panic!("initialize result")
        };
        assert_eq!(result.negotiated_protocol, kernel_v1::PROTOCOL_V1);
        assert_eq!(result.capabilities.max_aggregate_nodes, None);
        assert_eq!(result.capabilities.max_aggregate_elements, None);
        assert_eq!(result.capabilities.max_aggregate_depth, None);
        assert!(matches!(
            &messages[1],
            KernelSessionMessage::V1(kernel_v1::ServerMessage::Event(kernel_v1::EventEnvelope {
                event: Event::Status(openmat_protocol::StatusEvent {
                    status: KernelStatus::Idle
                }),
                ..
            }))
        ));
        assert_eq!(session.negotiated_protocol(), Some(ProtocolVersion::V1));

        let v0 = openmat_protocol::RequestEnvelope::new(
            "session-1",
            "mixed",
            openmat_protocol::Request::ListWorkspace(openmat_protocol::ListWorkspaceRequest {}),
        );
        assert_eq!(
            session
                .handle(KernelSessionRequest::V0(&v0))
                .expect_err("mid-session downgrade is forbidden")
                .category(),
            "protocol.mismatch"
        );
        assert_eq!(
            session
                .handle(KernelSessionRequest::Bootstrap(&bootstrap))
                .expect_err("bootstrap cannot repeat")
                .category(),
            "protocol.alreadyNegotiated"
        );
    }

    #[test]
    fn bootstrap_prefers_v2_and_locks_out_v1_and_v0() {
        let mut session = KernelSession::new("session-1", MockEngine::ordinary());
        let bootstrap = v2_bootstrap();
        let messages = session
            .handle(KernelSessionRequest::Bootstrap(&bootstrap))
            .unwrap();
        let KernelSessionMessage::Bootstrap(response) = &messages[0] else {
            panic!("bootstrap response")
        };
        assert_eq!(response.protocol, openmat_protocol::PROTOCOL_V0);
        let Some(BootstrapResponseResult::Initialize(result)) = &response.result else {
            panic!("initialize result")
        };
        assert_eq!(result.negotiated_protocol, kernel_v2::PROTOCOL_V2);
        assert_eq!(
            result.capabilities.max_aggregate_nodes,
            Some(kernel_v2::MAX_AGGREGATE_NODES)
        );
        assert!(matches!(
            &messages[1],
            KernelSessionMessage::V2(kernel_v2::ServerMessage::Event(
                kernel_v2::EventEnvelope {
                    protocol,
                    event: Event::Status(openmat_protocol::StatusEvent {
                        status: KernelStatus::Idle
                    }),
                    ..
                }
            )) if protocol == kernel_v2::PROTOCOL_V2
        ));
        assert_eq!(session.negotiated_protocol(), Some(ProtocolVersion::V2));

        let v1 = V1RequestEnvelope::new(
            "session-1",
            "mixed-v1",
            Request::ListWorkspace(openmat_protocol::ListWorkspaceRequest {}),
        );
        assert_eq!(
            session
                .handle(KernelSessionRequest::V1(&v1))
                .expect_err("mid-session v1 downgrade is forbidden")
                .category(),
            "protocol.mismatch"
        );
        let v0 = openmat_protocol::RequestEnvelope::new(
            "session-1",
            "mixed-v0",
            openmat_protocol::Request::ListWorkspace(openmat_protocol::ListWorkspaceRequest {}),
        );
        assert_eq!(
            session
                .handle(KernelSessionRequest::V0(&v0))
                .expect_err("mid-session v0 downgrade is forbidden")
                .category(),
            "protocol.mismatch"
        );
    }

    #[test]
    fn bootstrap_failures_are_structured_and_do_not_lock_the_session() {
        let mut session = KernelSession::new("session-1", MockEngine::ordinary());
        let no_common = bootstrap(
            vec!["openmat-kernel-v9".to_owned()],
            Capabilities::default(),
        );
        let messages = session
            .handle(KernelSessionRequest::Bootstrap(&no_common))
            .unwrap();
        let KernelSessionMessage::Bootstrap(response) = &messages[0] else {
            panic!("bootstrap failure")
        };
        assert_eq!(
            response.error.as_ref().expect("no common error").category,
            "protocol.noCommonVersion"
        );
        assert_eq!(session.negotiated_protocol(), None);

        let invalid = bootstrap(
            vec![kernel_v1::PROTOCOL_V1.to_owned()],
            Capabilities::default(),
        );
        let messages = session
            .handle(KernelSessionRequest::Bootstrap(&invalid))
            .unwrap();
        let KernelSessionMessage::Bootstrap(response) = &messages[0] else {
            panic!("bootstrap failure")
        };
        assert_eq!(
            response
                .error
                .as_ref()
                .expect("invalid capabilities")
                .category,
            "protocol.invalidCapabilities"
        );
        assert_eq!(session.negotiated_protocol(), None);

        let invalid_v2 = bootstrap(
            kernel_v2::production_protocol_offer(),
            client_capabilities(),
        );
        let messages = session
            .handle(KernelSessionRequest::Bootstrap(&invalid_v2))
            .unwrap();
        let KernelSessionMessage::Bootstrap(response) = &messages[0] else {
            panic!("v2 bootstrap failure")
        };
        assert_eq!(
            response
                .error
                .as_ref()
                .expect("invalid v2 capabilities")
                .category,
            "protocol.invalidCapabilities"
        );
        assert_eq!(session.negotiated_protocol(), None);

        session
            .handle(KernelSessionRequest::Bootstrap(&v1_bootstrap()))
            .unwrap();
        assert_eq!(session.negotiated_protocol(), Some(ProtocolVersion::V1));
    }

    #[test]
    fn v0_downgrade_uses_v0_after_the_bootstrap_response() {
        let mut session = KernelSession::new("session-1", MockEngine::ordinary());
        let bootstrap = bootstrap(
            vec![openmat_protocol::PROTOCOL_V0.to_owned()],
            Capabilities {
                max_preview_elements: 64,
                ..Capabilities::default()
            },
        );
        let messages = session
            .handle(KernelSessionRequest::Bootstrap(&bootstrap))
            .unwrap();
        assert!(matches!(messages[0], KernelSessionMessage::Bootstrap(_)));
        assert!(matches!(messages[1], KernelSessionMessage::V0(_)));
        assert_eq!(session.negotiated_protocol(), Some(ProtocolVersion::V0));
        let KernelSessionMessage::Bootstrap(response) = &messages[0] else {
            unreachable!()
        };
        let Some(BootstrapResponseResult::Initialize(result)) = &response.result else {
            panic!("v0 initialize result")
        };
        assert_eq!(result.capabilities.max_string_element_code_units, None);
        assert_eq!(result.capabilities.max_preview_code_units, None);
        assert_eq!(result.capabilities.max_aggregate_nodes, None);

        let v1 = V1RequestEnvelope::new(
            "session-1",
            "mixed",
            Request::ListWorkspace(openmat_protocol::ListWorkspaceRequest {}),
        );
        assert_eq!(
            session
                .handle(KernelSessionRequest::V1(&v1))
                .expect_err("mid-session upgrade is forbidden")
                .category(),
            "protocol.mismatch"
        );
    }

    #[test]
    fn v1_frames_use_request_aware_execute_and_inspect_mapping() {
        let mut session = KernelSession::new("session-1", MockEngine::ordinary());
        session
            .handle(KernelSessionRequest::Bootstrap(&v1_bootstrap()))
            .unwrap();
        let limits = negotiated_limits();

        let execute = V1RequestEnvelope::new(
            "session-1",
            "execute-1",
            Request::Execute(ExecuteRequest {
                code: "hello".to_owned(),
                source_name: "cell-1".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        let frames = session
            .handle_v1_frame(&kernel_v1::encode_request(&execute, &limits).unwrap())
            .unwrap();
        let response = decoded_response(&frames, &execute, &limits);
        assert!(matches!(
            response.result,
            Some(V1ResponseResult::Execute(ExecuteResult {
                interrupted: false
            }))
        ));
        assert!(frames.iter().filter_map(|frame| kernel_v1::decode_event(frame).ok()).any(
            |event| matches!(event.event, Event::Stream(StreamEvent { text, .. }) if text == "hello")
        ));

        let inspect = V1RequestEnvelope::new(
            "session-1",
            "inspect-1",
            Request::Inspect(InspectRequest {
                name: "answer".to_owned(),
                range: MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 1],
                },
                max_elements: 1,
            }),
        );
        let frames = session
            .handle_v1_frame(&kernel_v1::encode_request(&inspect, &limits).unwrap())
            .unwrap();
        assert!(matches!(
            decoded_response(&frames, &inspect, &limits).result,
            Some(V1ResponseResult::Inspect(_))
        ));

        let mut invalid_session = KernelSession::new(
            "session-1",
            MockEngine {
                invalid_preview: true,
                ..MockEngine::ordinary()
            },
        );
        invalid_session
            .handle(KernelSessionRequest::Bootstrap(&v1_bootstrap()))
            .unwrap();
        let frames = invalid_session
            .handle_v1_frame(&kernel_v1::encode_request(&inspect, &limits).unwrap())
            .unwrap();
        assert_eq!(
            decoded_response(&frames, &inspect, &limits)
                .error
                .expect("invalid preview error")
                .category,
            "engine.invalidPreview"
        );
    }

    #[test]
    fn v2_frames_map_execute_list_aggregate_inspect_and_shutdown_request_aware() {
        let range = kernel_v2::MatrixRange {
            start: vec![1, 1],
            size: vec![1, 1],
        };
        let preview = kernel_v2::InspectPreview::Aggregate(kernel_v2::AggregatePreview::Cell {
            dimensions: vec![1, 1],
            selected_range: range.clone(),
            items: vec![logical_exact(true)],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: kernel_v2::PreviewUsage {
                nodes: 2,
                elements: 2,
                code_units: 0,
                depth: 1,
            },
        });
        let mut session = KernelSession::new(
            "session-1",
            MockEngine {
                v2_preview: Some(preview),
                ..MockEngine::ordinary()
            },
        );
        session
            .handle(KernelSessionRequest::Bootstrap(&v2_bootstrap()))
            .unwrap();
        let limits = negotiated_v2_limits();

        let execute = kernel_v2::RequestEnvelope::new(
            "session-1",
            "execute-v2",
            kernel_v2::Request::Execute(ExecuteRequest {
                code: "hello-v2".to_owned(),
                source_name: "cell-v2".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        let frames = session
            .handle_v2_frame(&kernel_v2::encode_request(&execute, &limits).unwrap())
            .unwrap();
        assert!(matches!(
            decoded_v2_response(&frames, &execute, &limits).result,
            Some(kernel_v2::ResponseResult::Execute(ExecuteResult {
                interrupted: false
            }))
        ));

        let list = kernel_v2::RequestEnvelope::new(
            "session-1",
            "list-v2",
            kernel_v2::Request::ListWorkspace(openmat_protocol::ListWorkspaceRequest {}),
        );
        let frames = session
            .handle_v2_frame(&kernel_v2::encode_request(&list, &limits).unwrap())
            .unwrap();
        assert!(matches!(
            decoded_v2_response(&frames, &list, &limits).result,
            Some(kernel_v2::ResponseResult::ListWorkspace(_))
        ));

        let inspect = kernel_v2::RequestEnvelope::new(
            "session-1",
            "inspect-v2",
            kernel_v2::Request::Inspect(kernel_v2::InspectRequest {
                name: "answer".to_owned(),
                range,
                max_elements: 1,
            }),
        );
        let frames = session
            .handle_v2_frame(&kernel_v2::encode_request(&inspect, &limits).unwrap())
            .unwrap();
        let Some(kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Aggregate(
            kernel_v2::AggregatePreview::Cell { items, .. },
        ))) = decoded_v2_response(&frames, &inspect, &limits).result
        else {
            panic!("aggregate inspect response")
        };
        assert_eq!(items, vec![logical_exact(true)]);

        let shutdown = kernel_v2::RequestEnvelope::new(
            "session-1",
            "shutdown-v2",
            kernel_v2::Request::Shutdown(openmat_protocol::ShutdownRequest {}),
        );
        let frames = session
            .handle_v2_frame(&kernel_v2::encode_request(&shutdown, &limits).unwrap())
            .unwrap();
        assert!(matches!(
            decoded_v2_response(&frames, &shutdown, &limits).result,
            Some(kernel_v2::ResponseResult::Shutdown(_))
        ));
        assert_eq!(session.status(), KernelStatus::Dead);
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One lifecycle keeps revision assertions in wire order.
    fn v3_frames_version_reads_commit_atomically_and_reject_stale_writes() {
        let mut session = KernelSession::new("session-1", MockEngine::ordinary());
        let bootstrap_frames = session
            .handle(KernelSessionRequest::Bootstrap(&v3_bootstrap()))
            .unwrap();
        let KernelSessionMessage::Bootstrap(bootstrap_response) = &bootstrap_frames[0] else {
            panic!("bootstrap response")
        };
        assert!(matches!(
            bootstrap_response.result,
            Some(BootstrapResponseResult::Initialize(ref result))
                if result.negotiated_protocol == kernel_v3::PROTOCOL_V3
        ));
        let limits = negotiated_v2_limits();

        let inspect = kernel_v3::RequestEnvelope::new(
            "session-1",
            "inspect-v3-1",
            kernel_v3::Request::Inspect(kernel_v2::InspectRequest {
                name: "answer".to_owned(),
                range: kernel_v2::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 1],
                },
                max_elements: 1,
            }),
        );
        let frames = session
            .handle_v3_frame(&kernel_v3::encode_request(&inspect, &limits).unwrap())
            .unwrap();
        let Some(kernel_v3::ResponseResult::Inspect(initial)) =
            decoded_v3_response(&frames, &inspect, &limits).result
        else {
            panic!("versioned inspect response")
        };
        assert_eq!(initial.revision, 0);

        let write = kernel_v3::RequestEnvelope::new(
            "session-1",
            "set-v3-1",
            kernel_v3::Request::SetVariableElement(kernel_v3::SetVariableElementRequest {
                name: "answer".to_owned(),
                indices: vec![1, 1],
                value: kernel_v3::NumericScalar {
                    real: "9".to_owned(),
                    imaginary: "0".to_owned(),
                },
                expected_revision: initial.revision,
            }),
        );
        let frames = session
            .handle_v3_frame(&kernel_v3::encode_request(&write, &limits).unwrap())
            .unwrap();
        let Some(kernel_v3::ResponseResult::SetVariableElement(committed)) =
            decoded_v3_response(&frames, &write, &limits).result
        else {
            panic!("element mutation response")
        };
        assert_eq!(committed.revision, 1);

        let stale = kernel_v3::RequestEnvelope::new(
            "session-1",
            "set-v3-stale",
            kernel_v3::Request::SetVariableElement(kernel_v3::SetVariableElementRequest {
                name: "answer".to_owned(),
                indices: vec![1, 1],
                value: kernel_v3::NumericScalar {
                    real: "100".to_owned(),
                    imaginary: "0".to_owned(),
                },
                expected_revision: 0,
            }),
        );
        let frames = session
            .handle_v3_frame(&kernel_v3::encode_request(&stale, &limits).unwrap())
            .unwrap();
        assert_eq!(
            decoded_v3_response(&frames, &stale, &limits)
                .error
                .expect("stale write failure")
                .category,
            "workspace.revisionConflict"
        );

        let inspect_after = kernel_v3::RequestEnvelope::new(
            "session-1",
            "inspect-v3-2",
            kernel_v3::Request::Inspect(kernel_v2::InspectRequest {
                name: "answer".to_owned(),
                range: kernel_v2::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 1],
                },
                max_elements: 1,
            }),
        );
        let frames = session
            .handle_v3_frame(&kernel_v3::encode_request(&inspect_after, &limits).unwrap())
            .unwrap();
        let Some(kernel_v3::ResponseResult::Inspect(after)) =
            decoded_v3_response(&frames, &inspect_after, &limits).result
        else {
            panic!("post-mutation inspect response")
        };
        assert_eq!(after.revision, 1);
        let kernel_v2::InspectPreview::Matrix(matrix) = after.preview else {
            panic!("matrix preview")
        };
        assert!(matches!(
            matrix.values.as_slice(),
            [PreviewValue::Number { value }] if value.to_bits() == 9.0_f64.to_bits()
        ));
    }

    #[test]
    fn v2_frame_budget_rolls_back_only_complete_top_level_items() {
        let limits = kernel_v2::AggregateLimits::default();
        let request = kernel_v2::RequestEnvelope::new(
            "session-1",
            "large-inspect",
            kernel_v2::Request::Inspect(kernel_v2::InspectRequest {
                name: "large".to_owned(),
                range: kernel_v2::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 15],
                },
                max_elements: 15,
            }),
        );
        let exact = kernel_v2::ExactValue {
            class: "double".to_owned(),
            size: vec![1, 4_096],
            ndims: 2,
            numel: 4_096,
            complex: false,
            payload: kernel_v2::ExactPayload::Numeric {
                real: vec!["1.7976931348623157e308".to_owned(); 4_096],
                imag: vec!["0".to_owned(); 4_096],
            },
        };
        let response = kernel_v2::ResponseEnvelope::success(
            &request,
            "kernel-large",
            kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Aggregate(
                kernel_v2::AggregatePreview::Cell {
                    dimensions: vec![1, 15],
                    selected_range: kernel_v2::MatrixRange {
                        start: vec![1, 1],
                        size: vec![1, 15],
                    },
                    items: vec![exact; 15],
                    truncation: PreviewTruncation {
                        truncated: false,
                        omitted_elements: 0,
                    },
                    usage: kernel_v2::PreviewUsage {
                        nodes: 16,
                        elements: 61_455,
                        code_units: 0,
                        depth: 1,
                    },
                },
            )),
        );
        assert!(matches!(
            kernel_v2::encode_response_for_request(&response, &request, &limits),
            Err(kernel_v2::CodecError::FrameTooLarge { .. })
        ));

        let fitted = fit_v2_inspect_response(response, &request, &limits)
            .expect("whole-item frame truncation");
        let encoded = kernel_v2::encode_response_for_request(&fitted, &request, &limits)
            .expect("fitted frame");
        assert!(encoded.len() <= kernel_v2::MAX_JSON_FRAME_BYTES);
        let Some(kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Aggregate(
            kernel_v2::AggregatePreview::Cell {
                items,
                truncation,
                usage,
                ..
            },
        ))) = fitted.result
        else {
            panic!("fitted aggregate response")
        };
        assert!(items.len() < 15);
        assert_eq!(truncation.omitted_elements, 15 - items.len() as u64);
        assert_eq!(usage.nodes, 1 + items.len() as u64);
        assert_eq!(
            usage.elements,
            items.len() as u64 * (1 + exact_numel_for_test())
        );
    }

    #[test]
    fn v2_table_frame_budget_rolls_back_names_and_complete_variables_together() {
        let limits = kernel_v2::AggregateLimits::default();
        let request = kernel_v2::RequestEnvelope::new(
            "session-1",
            "large-table-inspect",
            kernel_v2::Request::Inspect(kernel_v2::InspectRequest {
                name: "large_table".to_owned(),
                range: kernel_v2::MatrixRange {
                    start: vec![1, 1],
                    size: vec![exact_numel_for_test(), 15],
                },
                max_elements: 15,
            }),
        );
        let exact = kernel_v2::ExactValue {
            class: "double".to_owned(),
            size: vec![exact_numel_for_test(), 1],
            ndims: 2,
            numel: exact_numel_for_test(),
            complex: false,
            payload: kernel_v2::ExactPayload::Numeric {
                real: vec!["1.7976931348623157e308".to_owned(); 4_096],
                imag: vec!["0".to_owned(); 4_096],
            },
        };
        let variable_names = (0..15)
            .map(|index| format!("V{index:02}"))
            .collect::<Vec<_>>();
        let response = kernel_v2::ResponseEnvelope::success(
            &request,
            "kernel-large-table",
            kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Aggregate(
                kernel_v2::AggregatePreview::Table {
                    dimensions: vec![exact_numel_for_test(), 15],
                    selected_range: kernel_v2::MatrixRange {
                        start: vec![1, 1],
                        size: vec![exact_numel_for_test(), 15],
                    },
                    variable_names,
                    variables: vec![exact; 15],
                    truncation: PreviewTruncation {
                        truncated: false,
                        omitted_elements: 0,
                    },
                    usage: kernel_v2::PreviewUsage {
                        nodes: 16,
                        elements: 15 * (1 + exact_numel_for_test()),
                        code_units: 45,
                        depth: 1,
                    },
                },
            )),
        );
        assert!(matches!(
            kernel_v2::encode_response_for_request(&response, &request, &limits),
            Err(kernel_v2::CodecError::FrameTooLarge { .. })
        ));

        let fitted = fit_v2_inspect_response(response, &request, &limits)
            .expect("whole-variable table frame truncation");
        let encoded = kernel_v2::encode_response_for_request(&fitted, &request, &limits)
            .expect("fitted table frame");
        assert!(encoded.len() <= kernel_v2::MAX_JSON_FRAME_BYTES);
        let Some(kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Aggregate(
            kernel_v2::AggregatePreview::Table {
                variable_names,
                variables,
                truncation,
                usage,
                ..
            },
        ))) = fitted.result
        else {
            panic!("fitted table aggregate response")
        };
        assert_eq!(variable_names.len(), variables.len());
        assert!(variables.len() < 15);
        assert_eq!(truncation.omitted_elements, 15 - variables.len() as u64);
        assert_eq!(usage.nodes, 1 + variables.len() as u64);
        assert_eq!(
            usage.elements,
            variables.len() as u64 * (1 + exact_numel_for_test())
        );
        assert_eq!(usage.code_units, variable_names.len() as u64 * 3);
    }

    const fn exact_numel_for_test() -> u64 {
        4_096
    }

    #[test]
    fn v1_interrupt_control_remains_independent_of_blocked_execution() {
        let started = Arc::new(AtomicBool::new(false));
        let mut session = KernelSession::new(
            "session-1",
            MockEngine {
                started: Some(Arc::clone(&started)),
                wait_for_interrupt: true,
                invalid_preview: false,
                v2_preview: None,
                value: 42.0,
            },
        );
        session
            .handle(KernelSessionRequest::Bootstrap(&v1_bootstrap()))
            .unwrap();
        let control = session.control().unwrap();
        let limits = negotiated_limits();
        let execute = V1RequestEnvelope::new(
            "session-1",
            "execute-blocked",
            Request::Execute(ExecuteRequest {
                code: "wait".to_owned(),
                source_name: "cell-1".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        let execute_json = kernel_v1::encode_request(&execute, &limits).unwrap();
        let execution = thread::spawn(move || session.handle_v1_frame(&execute_json).unwrap());
        let deadline = Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(started.load(Ordering::Acquire), "execution did not start");

        let interrupt = V1RequestEnvelope::new(
            "session-1",
            "interrupt-1",
            Request::Interrupt(InterruptRequest {}),
        );
        let frames = control
            .handle_v1_frame(&kernel_v1::encode_request(&interrupt, &limits).unwrap())
            .unwrap();
        let Some(V1ResponseResult::Interrupt(result)) =
            decoded_response(&frames, &interrupt, &limits).result
        else {
            panic!("interrupt result")
        };
        assert!(result.accepted);

        let frames = execution.join().expect("execution thread");
        let Some(V1ResponseResult::Execute(result)) =
            decoded_response(&frames, &execute, &limits).result
        else {
            panic!("execute result")
        };
        assert!(result.interrupted);
        assert_eq!(control.status(), KernelStatus::Idle);
    }

    #[test]
    fn v2_interrupt_control_remains_independent_of_blocked_execution() {
        let started = Arc::new(AtomicBool::new(false));
        let mut session = KernelSession::new(
            "session-1",
            MockEngine {
                started: Some(Arc::clone(&started)),
                wait_for_interrupt: true,
                invalid_preview: false,
                v2_preview: None,
                value: 42.0,
            },
        );
        session
            .handle(KernelSessionRequest::Bootstrap(&v2_bootstrap()))
            .unwrap();
        let control = session.control().unwrap();
        let limits = negotiated_v2_limits();
        let execute = kernel_v2::RequestEnvelope::new(
            "session-1",
            "execute-blocked-v2",
            kernel_v2::Request::Execute(ExecuteRequest {
                code: "wait".to_owned(),
                source_name: "cell-v2".to_owned(),
                mode: ExecutionMode::Repl,
            }),
        );
        let execute_json = kernel_v2::encode_request(&execute, &limits).unwrap();
        let execution = thread::spawn(move || session.handle_v2_frame(&execute_json).unwrap());
        let deadline = Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            started.load(Ordering::Acquire),
            "v2 execution did not start"
        );

        let interrupt = kernel_v2::RequestEnvelope::new(
            "session-1",
            "interrupt-v2",
            kernel_v2::Request::Interrupt(InterruptRequest {}),
        );
        let frames = control
            .handle_v2_frame(&kernel_v2::encode_request(&interrupt, &limits).unwrap())
            .unwrap();
        let Some(kernel_v2::ResponseResult::Interrupt(result)) =
            decoded_v2_response(&frames, &interrupt, &limits).result
        else {
            panic!("v2 interrupt result")
        };
        assert!(result.accepted);

        let frames = execution.join().expect("v2 execution thread");
        let Some(kernel_v2::ResponseResult::Execute(result)) =
            decoded_v2_response(&frames, &execute, &limits).result
        else {
            panic!("v2 execute result")
        };
        assert!(result.interrupted);
        assert_eq!(control.status(), KernelStatus::Idle);
    }
}
