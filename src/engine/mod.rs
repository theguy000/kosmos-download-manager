mod chunks;
mod coordinator;
mod model;
mod worker;

pub use chunks::{ChunkRange, calculate_chunks};
pub use coordinator::DownloadEngine;
pub use model::{
    ChunkSnapshot, DownloadAction, DownloadSnapshot, DownloadStatus, DuplicateChoice,
    DuplicatePrompt,
};
