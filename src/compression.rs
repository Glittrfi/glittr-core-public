use brotli::{
    enc::{backward_references::BrotliEncoderMode, BrotliEncoderParams},
    CompressorReader, Decompressor,
};
use std::io;
use std::io::Read;

pub trait Compression: Send + Sync {
    fn compress(&mut self, data: &[u8]) -> std::io::Result<Vec<u8>>;
    fn decompress(&mut self, data: &[u8]) -> std::io::Result<Vec<u8>>;
}

#[derive(Copy, Clone)]
pub struct Brotli {
    quality: i32, // 1-11
    lgwin: i32,   // 10-24 window size
    buffer_size: usize,
}

impl Brotli {
    pub fn new(quality: i32, lgwin: i32, buffer_size: usize) -> Self {
        assert!(quality >= 1);
        assert!(quality <= 11);
        assert!(lgwin >= 10);
        assert!(lgwin <= 24);

        Brotli {
            quality,
            lgwin,
            buffer_size,
        }
    }
}

impl Compression for Brotli {
    fn compress(&mut self, data: &[u8]) -> io::Result<Vec<u8>> {
        let params = BrotliEncoderParams {
            lgwin: self.lgwin,
            quality: self.quality,
            mode: BrotliEncoderMode::BROTLI_MODE_GENERIC,
            ..Default::default()
        };

        let mut comp = CompressorReader::with_params(data, self.buffer_size, &params);

        let mut compressed_data = Vec::new();
        comp.read_to_end(&mut compressed_data).unwrap();
        Ok(compressed_data)
    }

    fn decompress(&mut self, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut decomp = Decompressor::new(data, self.buffer_size);
        let mut decompressed_data = Vec::new();
        decomp.read_to_end(&mut decompressed_data).unwrap();
        Ok(decompressed_data)
    }
}
