//! On-demand body decoding. Bodies are stored as they came over the wire;
//! the inspector asks for a decoded copy when it shows one.
//!
//! This decodes whole stored bodies synchronously (flate2, brotli, zstd)
//! rather than streaming through async-compression: nothing here sits on the
//! forwarding path.

use std::io::Read;

/// Decoded output is capped so a compression bomb can't exhaust memory.
pub const DECODED_MAX: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unsupported content encoding {0:?}")]
    Unsupported(String),
    #[error("{encoding} body is corrupt: {source}")]
    Corrupt {
        encoding: String,
        #[source]
        source: std::io::Error,
    },
    #[error("decoded body is larger than {} MB", DECODED_MAX / 1024 / 1024)]
    TooLarge,
}

/// Undoes a `Content-Encoding` header value such as `gzip` or `deflate, br`
/// (codings are listed in the order they were applied).
pub fn decode(encoding: &str, body: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let mut data = body.to_vec();
    for coding in encoding.rsplit(',').map(|c| c.trim().to_ascii_lowercase()) {
        data = match coding.as_str() {
            "" | "identity" => data,
            "gzip" | "x-gzip" => read_all(&coding, flate2::read::MultiGzDecoder::new(&data[..]))?,
            "deflate" => inflate(&data)?,
            "br" => read_all(&coding, brotli::Decompressor::new(&data[..], 64 * 1024))?,
            "zstd" => read_all(
                &coding,
                zstd::stream::read::Decoder::new(&data[..]).map_err(|source| DecodeError::Corrupt {
                    encoding: coding.clone(),
                    source,
                })?,
            )?,
            _ => return Err(DecodeError::Unsupported(coding)),
        };
    }
    Ok(data)
}

/// HTTP `deflate` is meant to be zlib-wrapped, but plenty of servers send a
/// raw deflate stream; accept both.
fn inflate(data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    read_all("deflate", flate2::read::ZlibDecoder::new(data))
        .or_else(|_| read_all("deflate", flate2::read::DeflateDecoder::new(data)))
}

fn read_all(encoding: &str, reader: impl Read) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::new();
    reader
        .take(DECODED_MAX + 1)
        .read_to_end(&mut out)
        .map_err(|source| DecodeError::Corrupt {
            encoding: encoding.to_owned(),
            source,
        })?;
    if out.len() as u64 > DECODED_MAX {
        return Err(DecodeError::TooLarge);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    const BODY: &[u8] = br#"{"hello":"world","items":[1,2,3,4,5,6,7,8,9,10]}"#;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn decodes_each_supported_coding() {
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        zlib.write_all(BODY).unwrap();
        let mut raw = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        raw.write_all(BODY).unwrap();
        let mut br = Vec::new();
        brotli::CompressorWriter::new(&mut br, 4096, 5, 22)
            .write_all(BODY)
            .unwrap();
        let zstd = zstd::stream::encode_all(BODY, 3).unwrap();

        assert_eq!(decode("gzip", &gzip(BODY)).unwrap(), BODY);
        assert_eq!(decode("deflate", &zlib.finish().unwrap()).unwrap(), BODY);
        assert_eq!(decode("deflate", &raw.finish().unwrap()).unwrap(), BODY);
        assert_eq!(decode("br", &br).unwrap(), BODY);
        assert_eq!(decode("zstd", &zstd).unwrap(), BODY);
        assert_eq!(decode("identity", BODY).unwrap(), BODY);
    }

    #[test]
    fn stacked_codings_are_undone_in_reverse() {
        let twice = gzip(&gzip(BODY));
        assert_eq!(decode("gzip, gzip", &twice).unwrap(), BODY);
    }

    #[test]
    fn errors_are_reported() {
        assert!(matches!(decode("compress", BODY), Err(DecodeError::Unsupported(c)) if c == "compress"));
        assert!(matches!(decode("gzip", b"not gzip"), Err(DecodeError::Corrupt { .. })));
    }
}
