use serde::{Deserialize, Serialize};
use std::io;
use bytes::{BytesMut, Buf, BufMut};
use tokio_util::codec::{Decoder, Encoder};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Read;
use std::io::Write;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Packet {
    /// Keyboard input bytes sent from client to server
    ClientInput { data: Vec<u8> },
    /// Terminal window resize event
    ClientResize { rows: u16, cols: u16 },
    /// Ping message for latency/keepalive measurement
    Ping { timestamp: u64 },
    /// Pong response
    Pong { timestamp: u64 },
    /// Frame update sent from server to client every 20ms interval
    ServerFrame {
        seq: u64,
        data: Vec<u8>,
        compressed: bool,
    },
}

impl Packet {
    pub fn compress_data(data: &[u8]) -> (Vec<u8>, bool) {
        // Only compress if payload is larger than 128 bytes
        if data.len() > 128 {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
            if encoder.write_all(data).is_ok() {
                if let Ok(compressed) = encoder.finish() {
                    if compressed.len() < data.len() {
                        return (compressed, true);
                    }
                }
            }
        }
        (data.to_vec(), false)
    }

    pub fn decompress_data(data: &[u8], compressed: bool) -> io::Result<Vec<u8>> {
        if !compressed {
            return Ok(data.to_vec());
        }
        let mut decoder = GzDecoder::new(data);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        Ok(decompressed)
    }
}

/// Length-prefixed framing codec: [4-byte big endian length][bincode payload]
pub struct PacketCodec;

impl PacketCodec {
    pub fn new() -> Self {
        Self
    }
}

impl Decoder for PacketCodec {
    type Item = Packet;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }

        let mut length_bytes = [0u8; 4];
        length_bytes.copy_from_slice(&src[..4]);
        let length = u32::from_be_bytes(length_bytes) as usize;

        if src.len() < 4 + length {
            // Reserve additional capacity for remaining bytes
            src.reserve((4 + length) - src.len());
            return Ok(None);
        }

        src.advance(4);
        let payload = src.split_to(length);

        match bincode::deserialize::<Packet>(&payload) {
            Ok(packet) => Ok(Some(packet)),
            Err(e) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Deserialization error: {}", e),
            )),
        }
    }
}

impl Encoder<Packet> for PacketCodec {
    type Error = io::Error;

    fn encode(&mut self, item: Packet, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let serialized = bincode::serialize(&item).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Serialization error: {}", e))
        })?;

        let len = serialized.len() as u32;
        dst.reserve(4 + serialized.len());
        dst.put_u32(len);
        dst.put_slice(&serialized);
        Ok(())
    }
}
