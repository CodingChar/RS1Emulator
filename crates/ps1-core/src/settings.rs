/// Settings interface.
///
/// Layered settings (base + per-game), persisted via `ps1-wasm` to
/// `localStorage` or `IndexedDB`.
///
/// Mirrors DuckStation's `settings.cpp` / `layered_settings_interface.cpp`.
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Settings {
    layers: Vec<HashMap<String, String>>,
}

impl Settings {
    pub fn new() -> Self {
        Self { layers: vec![HashMap::new(), HashMap::new()] }
    }

    pub fn set(&mut self, layer: usize, key: &str, value: &str) {
        if layer < self.layers.len() {
            self.layers[layer].insert(key.to_string(), value.to_string());
        }
    }

    pub fn get(&self, layer: usize, key: &str) -> Option<&str> {
        self.layers.get(layer)?.get(key).map(|s| s.as_str())
    }

    pub fn get_or<'a>(&'a self, layer: usize, key: &str, default: &'a str) -> &'a str {
        self.get(layer, key).unwrap_or(default)
    }

    /// Search all layers from top to bottom.
    pub fn resolve(&self, key: &str) -> Option<&str> {
        for layer in self.layers.iter().rev() {
            if let Some(v) = layer.get(key) {
                return Some(v.as_str());
            }
        }
        None
    }
}
