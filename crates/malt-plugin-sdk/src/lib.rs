pub mod error;
pub mod host;
pub mod instance;
pub mod manifest;

pub use error::PluginError;
pub use host::PluginHost;
pub use manifest::PluginManifest;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_from_json() {
        let json =
            r#"{"name":"test","version":"0.1.0","permissions":["output"],"hooks":["on_output"]}"#;
        let manifest = PluginManifest::from_json(json).unwrap();
        assert_eq!(manifest.name, "test");
        assert!(manifest.has_permission("output"));
        assert!(!manifest.has_permission("network"));
        assert_eq!(manifest.effective_fuel(), 1_000_000);
    }

    #[test]
    fn manifest_custom_fuel() {
        let json =
            r#"{"name":"fast","version":"1.0","permissions":[],"hooks":[],"fuel_budget":500}"#;
        let manifest = PluginManifest::from_json(json).unwrap();
        assert_eq!(manifest.effective_fuel(), 500);
    }

    #[test]
    fn host_load_unload() {
        let mut host = PluginHost::new();
        assert_eq!(host.plugin_count(), 0);

        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            permissions: vec![],
            hooks: vec![],
            fuel_budget: None,
        };
        // Loading with invalid WASM should fail gracefully
        let result = host.load_plugin(b"not wasm", manifest);
        assert!(result.is_err());
        assert_eq!(host.plugin_count(), 0);
    }

    #[test]
    fn host_call_nonexistent() {
        let host = PluginHost::new();
        let result = host.call_plugin("nope", "func", &[]);
        assert!(matches!(result, Err(PluginError::PluginNotFound(_))));
    }

    #[test]
    fn load_minimal_wasm() {
        let wasm = wat::parse_str("(module)").unwrap();
        let manifest = PluginManifest {
            name: "minimal".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            permissions: vec![],
            hooks: vec![],
            fuel_budget: None,
        };
        let mut host = PluginHost::new();
        host.load_plugin(&wasm, manifest).unwrap();
        assert_eq!(host.plugin_count(), 1);
    }
}
