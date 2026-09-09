//! Server-owned graphics session attachment and kernel/hub boundary.

#![allow(clippy::missing_errors_doc)] // Public fallible methods return typed lifecycle errors.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, RwLock};

use openmat_plot_protocol::{
    AttachToken, ErrorCategory, Event, FigureSnapshot, FigureSummary, GraphicsLimits,
    ImplementationInfo, ProtocolFailure,
};
use serde_json::json;

const TOKEN_BYTES: usize = 32;

/// Immutable bytes borrowed from the session-owned graphics resource store.
#[derive(Clone, Debug)]
pub struct GraphicsBuffer {
    /// Opaque resource identifier.
    pub buffer_id: String,
    /// Complete canonical little-endian bytes.
    pub bytes: Arc<[u8]>,
}

/// Atomic initialize view. The hub must serialize this snapshot of revisions
/// with mutations and return an event cursor whose later deltas use exactly
/// those revisions as their bases.
#[derive(Clone, Debug)]
pub struct GraphicsHubInitialization {
    /// Host implementation identity.
    pub implementation: ImplementationInfo,
    /// Server/host limits before client negotiation.
    pub limits: GraphicsLimits,
    /// Every currently visible Figure and exact current revision.
    pub figures: Vec<FigureSummary>,
    /// Journal cursor immediately after the serialized initialize view.
    pub event_cursor: u64,
}

/// One session-journal event.
#[derive(Clone, Debug)]
pub struct SequencedGraphicsEvent {
    /// Strictly increasing hub-local journal sequence.
    pub sequence: u64,
    /// Typed protocol event.
    pub event: Event,
}

/// Close mutation outcome used to preserve response-before-event ordering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloseFigureOutcome {
    /// Exact close transaction revision.
    pub closed_revision: u64,
    /// Journal cursor including the close mutation, preventing duplicate emit.
    pub event_cursor: u64,
}

/// Atomic semantic-limit mutation outcome. The committed delta is retained in
/// the ordinary hub journal through `event_cursor` and is sent after the
/// correlated success response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetAxesLimitsOutcome {
    /// Exact committed Figure revision.
    pub committed_revision: u64,
    /// Journal cursor including the limits mutation.
    pub event_cursor: u64,
}

/// Atomic 3D camera mutation outcome retained in the ordinary delta journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetAxesCameraOutcome {
    pub committed_revision: u64,
    pub event_cursor: u64,
}

/// Lease-release outcome. `reclaimed` is true only after both live Figure
/// references and all client leases reached zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferLeaseRelease {
    /// Whether the immutable resource was reclaimed.
    pub reclaimed: bool,
}

/// Structured non-secret failure returned by a kernel-owned hub adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphicsHubError {
    category: ErrorCategory,
    message: String,
}

impl GraphicsHubError {
    /// Creates one hub failure. The message must not contain an attachment
    /// token.
    #[must_use]
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    /// Returns the stable category.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    /// Returns non-secret diagnostic prose.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GraphicsHubError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl Error for GraphicsHubError {}

impl From<GraphicsHubError> for ProtocolFailure {
    fn from(error: GraphicsHubError) -> Self {
        Self::new(error.category, error.message)
    }
}

/// Shared kernel/session adapter consumed by the versioned graphics endpoints.
///
/// Implementations own the Graphics Object Model, revision journal, immutable
/// resource store, and serialized mutation boundary. The Server calls
/// `get_buffer(..., true)` only for a connection's first idempotent lease and
/// calls `release_buffer_lease` exactly once for every such lease on explicit
/// release or disconnect.
pub trait GraphicsSessionHub: Send + Sync + 'static {
    /// Serializes initialization with mutations and returns exact delta bases.
    fn initialize(&self) -> Result<GraphicsHubInitialization, GraphicsHubError>;

    /// Returns one complete authoritative snapshot.
    fn get_snapshot(&self, figure_id: &str) -> Result<FigureSnapshot, GraphicsHubError>;

    /// Returns immutable bytes. `acquire_lease` is true only when this active
    /// connection did not already lease `buffer_id`.
    fn get_buffer(
        &self,
        buffer_id: &str,
        acquire_lease: bool,
    ) -> Result<GraphicsBuffer, GraphicsHubError>;

    /// Removes one previously acquired client lease.
    fn release_buffer_lease(&self, buffer_id: &str)
    -> Result<BufferLeaseRelease, GraphicsHubError>;

    /// Performs the kernel-equivalent close mutation under exact revision
    /// compare-and-swap semantics.
    fn close_figure(
        &self,
        figure_id: &str,
        expected_revision: u64,
    ) -> Result<CloseFigureOutcome, GraphicsHubError>;

    /// Atomically applies both semantic limit pairs to one Figure Axes under
    /// exact revision compare-and-swap semantics.
    fn set_axes_limits(
        &self,
        figure_id: &str,
        axes_id: Option<&str>,
        expected_revision: u64,
        x_limits: [f64; 2],
        y_limits: [f64; 2],
    ) -> Result<SetAxesLimitsOutcome, GraphicsHubError>;

    /// Atomically applies one 3D view/scale gesture under revision compare-and-swap.
    fn set_axes_camera(
        &self,
        figure_id: &str,
        axes_id: Option<&str>,
        expected_revision: u64,
        view: [f64; 2],
        camera_scale: f64,
    ) -> Result<SetAxesCameraOutcome, GraphicsHubError>;

    /// Returns an authoritative complete snapshot after a client gap.
    fn resync_figure(
        &self,
        figure_id: &str,
        known_revision: u64,
    ) -> Result<FigureSnapshot, GraphicsHubError>;

    /// Returns journal events strictly after `cursor`, in sequence order.
    fn events_after(&self, cursor: u64) -> Result<Vec<SequencedGraphicsEvent>, GraphicsHubError>;
}

#[derive(Clone)]
struct SecretToken([u8; TOKEN_BYTES]);

impl SecretToken {
    fn generate() -> io::Result<Self> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        fill_secure_random(&mut bytes)?;
        Ok(Self(bytes))
    }

    fn encode(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(TOKEN_BYTES * 2);
        for byte in self.0 {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        encoded
    }

    fn matches_wire(&self, wire: &str) -> bool {
        let Some(candidate) = decode_hex_token(wire) else {
            return false;
        };
        self.0
            .iter()
            .zip(candidate)
            .fold(0_u8, |difference, (left, right)| {
                difference | (*left ^ right)
            })
            == 0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken([REDACTED])")
    }
}

fn decode_hex_token(wire: &str) -> Option<[u8; TOKEN_BYTES]> {
    if wire.len() != TOKEN_BYTES * 2 || !wire.is_ascii() {
        return None;
    }
    let mut decoded = [0_u8; TOKEN_BYTES];
    for (index, pair) in wire.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(decoded)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn fill_secure_random(bytes: &mut [u8]) -> io::Result<()> {
    getrandom::fill(bytes).map_err(|error| io::Error::other(error.to_string()))
}

struct EntryState {
    token: SecretToken,
    token_generation: u64,
    active_client: Option<u64>,
    next_client: u64,
    alive: bool,
}

struct RegistryEntry {
    session_id: String,
    hub: Arc<dyn GraphicsSessionHub>,
    state: Mutex<EntryState>,
}

/// Server-owned registration returned to the kernel connection adapter.
/// Dropping this handle does not implicitly tear down state: kernel shutdown or
/// socket disconnect must call [`GraphicsSessionRegistry::unregister_session`]
/// explicitly so lifecycle ordering is visible and testable.
#[derive(Clone)]
pub struct GraphicsRegistration {
    entry: Arc<RegistryEntry>,
}

impl fmt::Debug for GraphicsRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GraphicsRegistration")
            .field("session_id", &self.entry.session_id)
            .finish_non_exhaustive()
    }
}

/// Discovery MIME payload. The token remains redacted in `Debug` output.
#[derive(Clone)]
pub struct GraphicsDiscovery {
    /// Exactly four for newly announced Figures.
    pub schema_version: u64,
    /// Exactly `openmat-graphics-v4` for newly announced Figures.
    pub graphics_protocol: String,
    /// Origin-relative endpoint.
    pub endpoint: String,
    /// Server-owned capability.
    pub attach_token: AttachToken,
    /// Newly discovered Figure identifier.
    pub figure_id: String,
    /// Exact Figure revision.
    pub revision: u64,
}

impl fmt::Debug for GraphicsDiscovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GraphicsDiscovery")
            .field("schema_version", &self.schema_version)
            .field("graphics_protocol", &self.graphics_protocol)
            .field("endpoint", &self.endpoint)
            .field("attach_token", &self.attach_token)
            .field("figure_id", &self.figure_id)
            .field("revision", &self.revision)
            .finish()
    }
}

impl GraphicsRegistration {
    /// Creates the MIME payload for first discovery, explicit reattachment, or
    /// token rotation. Ordinary Figure changes must use deltas instead.
    pub fn discovery(
        &self,
        figure_id: impl Into<String>,
        revision: u64,
    ) -> Result<GraphicsDiscovery, GraphicsRegistryError> {
        let state = self.entry.state.lock().map_err(lock_poisoned)?;
        if !state.alive {
            return Err(GraphicsRegistryError::SessionClosed);
        }
        Ok(GraphicsDiscovery {
            schema_version: 4,
            graphics_protocol: openmat_plot_protocol::PROTOCOL_V4.to_owned(),
            endpoint: openmat_plot_protocol::GraphicsProtocol::V4
                .endpoint()
                .to_owned(),
            attach_token: AttachToken::new(state.token.encode()),
            figure_id: figure_id.into(),
            revision,
        })
    }
}

/// Registry lifecycle failure. No variant stores or formats an attachment
/// token.
#[derive(Debug)]
pub enum GraphicsRegistryError {
    /// CSPRNG failure prevented token creation/rotation.
    Entropy(io::Error),
    /// A live session identifier was registered twice.
    DuplicateSession,
    /// Registration no longer names a live session.
    SessionClosed,
    /// Internal synchronization was poisoned.
    Poisoned,
}

impl fmt::Display for GraphicsRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entropy(error) => write!(formatter, "graphics token generation failed: {error}"),
            Self::DuplicateSession => formatter.write_str("graphics session is already registered"),
            Self::SessionClosed => formatter.write_str("graphics session is closed"),
            Self::Poisoned => formatter.write_str("graphics registry synchronization failed"),
        }
    }
}

impl Error for GraphicsRegistryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Entropy(error) => Some(error),
            Self::DuplicateSession | Self::SessionClosed | Self::Poisoned => None,
        }
    }
}

/// Thread-safe Server-owned graphics session registry.
#[derive(Clone, Default)]
pub struct GraphicsSessionRegistry {
    sessions: Arc<RwLock<HashMap<String, Arc<RegistryEntry>>>>,
}

impl fmt::Debug for GraphicsSessionRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GraphicsSessionRegistry")
            .finish_non_exhaustive()
    }
}

impl GraphicsSessionRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one hub before any discovery display is emitted. Duplicate
    /// live session IDs are rejected.
    pub fn register_session(
        &self,
        session_id: impl Into<String>,
        hub: Arc<dyn GraphicsSessionHub>,
    ) -> Result<GraphicsRegistration, GraphicsRegistryError> {
        let session_id = session_id.into();
        if session_id.is_empty() {
            return Err(GraphicsRegistryError::SessionClosed);
        }
        let token = SecretToken::generate().map_err(GraphicsRegistryError::Entropy)?;
        let entry = Arc::new(RegistryEntry {
            session_id: session_id.clone(),
            hub,
            state: Mutex::new(EntryState {
                token,
                token_generation: 1,
                active_client: None,
                next_client: 1,
                alive: true,
            }),
        });
        let mut sessions = self.sessions.write().map_err(lock_poisoned)?;
        if sessions.contains_key(&session_id) {
            return Err(GraphicsRegistryError::DuplicateSession);
        }
        sessions.insert(session_id, Arc::clone(&entry));
        Ok(GraphicsRegistration { entry })
    }

    /// Invalidates the token, marks an active renderer terminal, and removes
    /// the lookup entry. The graphics socket observes `sessionClosed`; its drop
    /// path releases all client leases. Kernel socket disconnect and orderly
    /// kernel shutdown both call this operation.
    pub fn unregister_session(
        &self,
        registration: &GraphicsRegistration,
    ) -> Result<bool, GraphicsRegistryError> {
        {
            let mut state = registration.entry.state.lock().map_err(lock_poisoned)?;
            if !state.alive {
                return Ok(false);
            }
            state.alive = false;
        }
        let mut sessions = self.sessions.write().map_err(lock_poisoned)?;
        if sessions
            .get(&registration.entry.session_id)
            .is_some_and(|entry| Arc::ptr_eq(entry, &registration.entry))
        {
            sessions.remove(&registration.entry.session_id);
        }
        Ok(true)
    }

    /// Rotates the Server-owned token atomically. An active old-token client is
    /// made stale and receives `sessionClosed` before a new attachment can win.
    pub fn rotate_token(
        &self,
        registration: &GraphicsRegistration,
    ) -> Result<(), GraphicsRegistryError> {
        let token = SecretToken::generate().map_err(GraphicsRegistryError::Entropy)?;
        let mut state = registration.entry.state.lock().map_err(lock_poisoned)?;
        if !state.alive {
            return Err(GraphicsRegistryError::SessionClosed);
        }
        state.token = token;
        state.token_generation = state
            .token_generation
            .checked_add(1)
            .ok_or(GraphicsRegistryError::SessionClosed)?;
        Ok(())
    }

    pub(crate) fn attach(
        &self,
        session_id: &str,
        token: &AttachToken,
    ) -> Result<GraphicsAttachment, ProtocolFailure> {
        let entry = self
            .sessions
            .read()
            .map_err(|_| registry_failure())?
            .get(session_id)
            .cloned()
            .ok_or_else(unauthorized)?;
        let mut state = entry.state.lock().map_err(|_| registry_failure())?;
        if !state.alive {
            return Err(ProtocolFailure::new(
                ErrorCategory::SessionClosed,
                "graphics session is closed",
            ));
        }
        if !state.token.matches_wire(token.expose_secret()) {
            return Err(unauthorized());
        }
        if state.active_client.is_some() {
            return Err(ProtocolFailure::new(
                ErrorCategory::SessionInUse,
                "graphics session already has an active client",
            ));
        }
        let client_id = state.next_client;
        state.next_client = state.next_client.checked_add(1).ok_or_else(|| {
            ProtocolFailure::new(
                ErrorCategory::SessionClosed,
                "graphics client IDs exhausted",
            )
        })?;
        state.active_client = Some(client_id);
        let token_generation = state.token_generation;
        drop(state);
        Ok(GraphicsAttachment {
            entry,
            client_id,
            token_generation,
            leased_buffers: HashSet::new(),
        })
    }
}

pub(crate) struct GraphicsAttachment {
    entry: Arc<RegistryEntry>,
    client_id: u64,
    token_generation: u64,
    leased_buffers: HashSet<String>,
}

impl fmt::Debug for GraphicsAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GraphicsAttachment")
            .field("session_id", &self.entry.session_id)
            .field("client_id", &self.client_id)
            .field("leased_buffer_count", &self.leased_buffers.len())
            .finish_non_exhaustive()
    }
}

impl GraphicsAttachment {
    pub(crate) fn session_id(&self) -> &str {
        &self.entry.session_id
    }

    pub(crate) fn hub(&self) -> &dyn GraphicsSessionHub {
        self.entry.hub.as_ref()
    }

    pub(crate) fn is_live(&self) -> bool {
        self.entry.state.lock().is_ok_and(|state| {
            state.alive
                && state.token_generation == self.token_generation
                && state.active_client == Some(self.client_id)
        })
    }

    pub(crate) fn get_buffer(
        &mut self,
        buffer_id: &str,
    ) -> Result<(GraphicsBuffer, bool), GraphicsHubError> {
        let newly_leased = self.leased_buffers.insert(buffer_id.to_owned());
        match self.entry.hub.get_buffer(buffer_id, newly_leased) {
            Ok(buffer) => Ok((buffer, newly_leased)),
            Err(error) => {
                if newly_leased {
                    self.leased_buffers.remove(buffer_id);
                }
                Err(error)
            }
        }
    }

    pub(crate) fn cancel_new_lease(&mut self, buffer_id: &str, newly_leased: bool) {
        if newly_leased && self.leased_buffers.remove(buffer_id) {
            let _ = self.entry.hub.release_buffer_lease(buffer_id);
        }
    }

    pub(crate) fn release_buffer(
        &mut self,
        buffer_id: &str,
    ) -> Result<(bool, BufferLeaseRelease), GraphicsHubError> {
        if !self.leased_buffers.remove(buffer_id) {
            return Ok((false, BufferLeaseRelease { reclaimed: false }));
        }
        match self.entry.hub.release_buffer_lease(buffer_id) {
            Ok(outcome) => Ok((true, outcome)),
            Err(error) => {
                self.leased_buffers.insert(buffer_id.to_owned());
                Err(error)
            }
        }
    }
}

impl Drop for GraphicsAttachment {
    fn drop(&mut self) {
        for buffer_id in self.leased_buffers.drain() {
            let _ = self.entry.hub.release_buffer_lease(&buffer_id);
        }
        if let Ok(mut state) = self.entry.state.lock()
            && state.active_client == Some(self.client_id)
        {
            state.active_client = None;
        }
    }
}

fn unauthorized() -> ProtocolFailure {
    ProtocolFailure::new(
        ErrorCategory::Unauthorized,
        "graphics attachment was not authorized",
    )
}

fn registry_failure() -> ProtocolFailure {
    ProtocolFailure::new(
        ErrorCategory::SessionClosed,
        "graphics registry is unavailable",
    )
}

fn lock_poisoned<T>(_error: std::sync::PoisonError<T>) -> GraphicsRegistryError {
    GraphicsRegistryError::Poisoned
}

/// Serializes a discovery payload for the existing MIME display boundary.
/// This helper never formats the token into diagnostics.
pub fn encode_discovery(discovery: &GraphicsDiscovery) -> Result<String, serde_json::Error> {
    serde_json::to_string(&json!({
        "schemaVersion": discovery.schema_version,
        "graphicsProtocol": discovery.graphics_protocol,
        "endpoint": discovery.endpoint,
        "attachToken": discovery.attach_token.expose_secret(),
        "figureId": discovery.figure_id,
        "revision": discovery.revision,
    }))
}

/// Returns the exact existing MIME string negotiated on the kernel protocol.
#[must_use]
pub const fn discovery_mime_type() -> &'static str {
    "application/vnd.openmat.figure+json"
}

/// Returns a redacted diagnostic projection useful for tests/telemetry.
#[must_use]
pub fn redacted_discovery_summary(discovery: &GraphicsDiscovery) -> serde_json::Value {
    json!({
        "schemaVersion": discovery.schema_version,
        "graphicsProtocol": discovery.graphics_protocol,
        "endpoint": discovery.endpoint,
        "attachToken": "[REDACTED]",
        "figureId": discovery.figure_id,
        "revision": discovery.revision,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct EmptyHub {
        releases: AtomicUsize,
    }

    impl GraphicsSessionHub for EmptyHub {
        fn initialize(&self) -> Result<GraphicsHubInitialization, GraphicsHubError> {
            Ok(GraphicsHubInitialization {
                implementation: ImplementationInfo {
                    name: "fixture".to_owned(),
                    version: "1".to_owned(),
                },
                limits: GraphicsLimits::default(),
                figures: vec![],
                event_cursor: 0,
            })
        }

        fn get_snapshot(&self, _figure_id: &str) -> Result<FigureSnapshot, GraphicsHubError> {
            Err(GraphicsHubError::new(
                ErrorCategory::UnknownFigure,
                "unknown fixture Figure",
            ))
        }

        fn get_buffer(
            &self,
            buffer_id: &str,
            _acquire_lease: bool,
        ) -> Result<GraphicsBuffer, GraphicsHubError> {
            Ok(GraphicsBuffer {
                buffer_id: buffer_id.to_owned(),
                bytes: Arc::from([1_u8, 2, 3]),
            })
        }

        fn release_buffer_lease(
            &self,
            _buffer_id: &str,
        ) -> Result<BufferLeaseRelease, GraphicsHubError> {
            self.releases.fetch_add(1, Ordering::Relaxed);
            Ok(BufferLeaseRelease { reclaimed: true })
        }

        fn close_figure(
            &self,
            _figure_id: &str,
            _expected_revision: u64,
        ) -> Result<CloseFigureOutcome, GraphicsHubError> {
            unreachable!()
        }

        fn set_axes_limits(
            &self,
            _figure_id: &str,
            _axes_id: Option<&str>,
            _expected_revision: u64,
            _x_limits: [f64; 2],
            _y_limits: [f64; 2],
        ) -> Result<SetAxesLimitsOutcome, GraphicsHubError> {
            unreachable!()
        }

        fn set_axes_camera(
            &self,
            _figure_id: &str,
            _axes_id: Option<&str>,
            _expected_revision: u64,
            _view: [f64; 2],
            _camera_scale: f64,
        ) -> Result<SetAxesCameraOutcome, GraphicsHubError> {
            unreachable!()
        }

        fn resync_figure(
            &self,
            _figure_id: &str,
            _known_revision: u64,
        ) -> Result<FigureSnapshot, GraphicsHubError> {
            unreachable!()
        }

        fn events_after(
            &self,
            _cursor: u64,
        ) -> Result<Vec<SequencedGraphicsEvent>, GraphicsHubError> {
            Ok(vec![])
        }
    }

    #[test]
    fn token_is_256_bit_hex_redacted_and_reusable_after_disconnect() {
        let registry = GraphicsSessionRegistry::new();
        let hub = Arc::new(EmptyHub {
            releases: AtomicUsize::new(0),
        });
        let registration = registry.register_session("session-1", hub.clone()).unwrap();
        let discovery = registration.discovery("figure-1", 1).unwrap();
        assert_eq!(
            discovery.graphics_protocol,
            openmat_plot_protocol::PROTOCOL_V4
        );
        assert_eq!(discovery.schema_version, 4);
        assert_eq!(discovery.endpoint, "/graphics/v4");
        assert_eq!(discovery.attach_token.expose_secret().len(), 64);
        assert!(!format!("{discovery:?}").contains(discovery.attach_token.expose_secret()));

        let mut attachment = registry
            .attach("session-1", &discovery.attach_token)
            .unwrap();
        attachment.get_buffer("buffer-1").unwrap();
        assert_eq!(
            registry
                .attach("session-1", &discovery.attach_token)
                .unwrap_err()
                .category,
            ErrorCategory::SessionInUse
        );
        drop(attachment);
        assert_eq!(hub.releases.load(Ordering::Relaxed), 1);
        assert!(
            registry
                .attach("session-1", &discovery.attach_token)
                .is_ok()
        );
    }

    #[test]
    fn wrong_token_never_appears_in_failure_or_debug() {
        let registry = GraphicsSessionRegistry::new();
        let registration = registry
            .register_session(
                "session-1",
                Arc::new(EmptyHub {
                    releases: AtomicUsize::new(0),
                }),
            )
            .unwrap();
        let wrong = AttachToken::new("secret-that-must-not-appear");
        let error = registry.attach("session-1", &wrong).unwrap_err();
        assert_eq!(error.category, ErrorCategory::Unauthorized);
        assert!(!format!("{error:?} {error:?}").contains(wrong.expose_secret()));
        assert!(registry.unregister_session(&registration).unwrap());
    }

    #[test]
    fn unregister_and_rotation_invalidate_old_attachments_and_tokens() {
        let registry = GraphicsSessionRegistry::new();
        let registration = registry
            .register_session(
                "session-1",
                Arc::new(EmptyHub {
                    releases: AtomicUsize::new(0),
                }),
            )
            .unwrap();
        let old = registration.discovery("figure-1", 1).unwrap();
        let attachment = registry.attach("session-1", &old.attach_token).unwrap();
        registry.rotate_token(&registration).unwrap();
        assert!(!attachment.is_live());
        drop(attachment);
        assert_eq!(
            registry
                .attach("session-1", &old.attach_token)
                .unwrap_err()
                .category,
            ErrorCategory::Unauthorized
        );
        assert!(registry.unregister_session(&registration).unwrap());
        assert!(!registry.unregister_session(&registration).unwrap());
    }
}
