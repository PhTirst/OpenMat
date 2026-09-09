use std::fmt;

/// A registry-local class identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClassId(u64);

impl ClassId {
    pub(crate) const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the registry-local numeric identity for diagnostics and runtime tables.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ClassId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "class#{}", self.0)
    }
}

/// A store-local identity for allocated object state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId(u64);

impl ObjectId {
    pub(crate) const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the store-local numeric identity for diagnostics and runtime tables.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "object#{}", self.0)
    }
}

/// The stable key of an instance property slot.
///
/// The declaring class is part of the key so future compatibility work can
/// represent otherwise-hidden names without changing the store layout.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PropertyKey {
    declaring_class: ClassId,
    name: String,
}

impl PropertyKey {
    pub(crate) fn new(declaring_class: ClassId, name: impl Into<String>) -> Self {
        Self {
            declaring_class,
            name: name.into(),
        }
    }

    #[must_use]
    pub const fn declaring_class(&self) -> ClassId {
        self.declaring_class
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}
