use crate::error::PluginError;
use crate::instance::PluginInstance;
use crate::manifest::PluginManifest;
use std::collections::HashMap;

pub struct PluginHost {
    plugins: HashMap<String, PluginInstance>,
}

impl PluginHost {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn load_plugin(
        &mut self,
        wasm_bytes: &[u8],
        manifest: PluginManifest,
    ) -> Result<(), PluginError> {
        let name = manifest.name.clone();
        let instance = PluginInstance::load(wasm_bytes, manifest)?;
        self.plugins.insert(name, instance);
        Ok(())
    }

    pub fn unload_plugin(&mut self, name: &str) {
        self.plugins.remove(name);
    }

    pub fn call_plugin(
        &self,
        name: &str,
        func: &str,
        input: &[u8],
    ) -> Result<Vec<u8>, PluginError> {
        let instance = self
            .plugins
            .get(name)
            .ok_or_else(|| PluginError::PluginNotFound(name.to_string()))?;
        instance.call(func, input)
    }

    pub fn list_plugins(&self) -> Vec<&PluginManifest> {
        self.plugins.values().map(|p| p.manifest()).collect()
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}
