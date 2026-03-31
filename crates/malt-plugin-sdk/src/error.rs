#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    #[error("plugin not found: {0}")]
    PluginNotFound(String),
    #[error("failed to load plugin: {0}")]
    LoadFailed(String),
    #[error("failed to instantiate: {0}")]
    InstantiationFailed(String),
    #[error("function not found: {0}: {1}")]
    FunctionNotFound(String, String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
    #[error("fuel exhausted during {0}")]
    FuelExhausted(String),
    #[error("fuel error: {0}")]
    FuelError(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
}
