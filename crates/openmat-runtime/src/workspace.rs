use std::collections::BTreeMap;

use openmat_value::Value;

/// Persistent name-to-value bindings for one sequential language workspace.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspace {
    bindings: BTreeMap<String, Value>,
}

impl Workspace {
    /// Creates an empty workspace.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bindings: BTreeMap::new(),
        }
    }

    /// Returns a binding without changing the workspace.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.bindings.get(name)
    }

    /// Returns a mutable binding.
    ///
    /// Mutating an array element through the returned value preserves language
    /// value semantics: a previously cloned array buffer detaches on its first
    /// write, while opaque object handles retain identity.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.bindings.get_mut(name)
    }

    /// Creates or replaces a binding and returns its old value.
    pub fn insert(&mut self, name: impl Into<String>, value: Value) -> Option<Value> {
        self.bindings.insert(name.into(), value)
    }

    /// Removes a binding.
    pub fn remove(&mut self, name: &str) -> Option<Value> {
        self.bindings.remove(name)
    }

    /// Removes every workspace binding.
    pub fn clear(&mut self) {
        self.bindings.clear();
    }

    /// Returns whether a binding exists.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.bindings.contains_key(name)
    }

    /// Returns the number of bindings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Returns whether the workspace is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Iterates over bindings in deterministic name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.bindings
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }
}
