#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RendererError {
    #[error("client not found: {0}")]
    ClientNotFound(u64),

    #[error("frame too large: {size} bytes exceeds {max} byte limit")]
    FrameTooLarge { size: usize, max: usize },
}
