#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CompatError {
    #[error("invalid grid dimensions: {cols}x{rows}")]
    InvalidDimensions { cols: u16, rows: u16 },
}
