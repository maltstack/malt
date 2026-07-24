use crate::error::PluginError;
use wasmtime::{Engine, Linker, Module, Store};

pub struct PluginInstance {
    engine: Engine,
    module: Module,
    manifest: crate::manifest::PluginManifest,
}

impl PluginInstance {
    pub fn load(
        wasm_bytes: &[u8],
        manifest: crate::manifest::PluginManifest,
    ) -> Result<Self, PluginError> {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|e| PluginError::LoadFailed(e.to_string()))?;
        let module =
            Module::new(&engine, wasm_bytes).map_err(|e| PluginError::LoadFailed(e.to_string()))?;
        Ok(Self {
            engine,
            module,
            manifest,
        })
    }

    pub fn call(&self, func_name: &str, _input: &[u8]) -> Result<Vec<u8>, PluginError> {
        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(self.manifest.effective_fuel())
            .map_err(|e| PluginError::FuelError(e.to_string()))?;

        let linker = Linker::new(&self.engine);
        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| PluginError::InstantiationFailed(e.to_string()))?;

        // Get the exported function
        let func = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, func_name)
            .map_err(|e| PluginError::FunctionNotFound(func_name.to_string(), e.to_string()))?;

        // Scaffold: call with zero args. Real plugin ABI with shared memory comes later.
        let _result = func.call(&mut store, (0, 0)).map_err(|e| {
            if e.to_string().contains("fuel") {
                PluginError::FuelExhausted(func_name.to_string())
            } else {
                PluginError::ExecutionFailed(e.to_string())
            }
        })?;

        let remaining = store.get_fuel().unwrap_or(0);
        tracing::debug!(
            func_name,
            fuel_used = self.manifest.effective_fuel() - remaining,
            "plugin call completed"
        );

        Ok(Vec::new()) // Real output via shared memory later
    }

    pub fn manifest(&self) -> &crate::manifest::PluginManifest {
        &self.manifest
    }
}
