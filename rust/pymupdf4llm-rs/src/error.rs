use thiserror::Error;

pub type Result<T> = std::result::Result<T, PdfError>;

#[derive(Debug, Error)]
pub enum PdfError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid PDF header or missing %PDF- signature")]
    InvalidHeader,

    #[error("Cross-reference table or stream missing or corrupted: {0}")]
    InvalidXref(String),

    #[error("PDF syntax error at byte offset {offset}: {message}")]
    Syntax { offset: usize, message: String },

    #[error("Stream decompression failed ({filter}): {message}")]
    Decompress { filter: String, message: String },

    #[error("Encrypted PDF documents require a password or are unsupported in current mode")]
    EncryptedDocument,

    #[error("Page index {index} out of bounds (document has {total} pages)")]
    PageOutOfBounds { index: usize, total: usize },

    #[error("Object reference {obj_num} {gen_num} R not found")]
    MissingObject { obj_num: u32, gen_num: u16 },

    #[error("Image encoding error: {0}")]
    ImageEncode(String),

    #[error("Async task execution error: {0}")]
    TaskJoin(String),
}
