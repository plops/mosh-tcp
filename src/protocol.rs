use bytes::{Buf, BufMut, BytesMut};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{self, Read, Write};
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Binary serialization tag format:
    /// Tag 1: ClientInput { len: u32, data }
    /// Tag 2: ClientResize { rows: u16, cols: u16 }
    /// Tag 3: Ping { timestamp: u64 }
    /// Tag 4: Pong { timestamp: u64 }
    /// Tag 5: ServerFrame { seq: u64, compressed: u8, len: u32, data }
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Packet::ClientInput { data } => {
                buf.push(1u8);
                buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
                buf.extend_from_slice(data);
            }
            Packet::ClientResize { rows, cols } => {
                buf.push(2u8);
                buf.extend_from_slice(&rows.to_be_bytes());
                buf.extend_from_slice(&cols.to_be_bytes());
            }
            Packet::Ping { timestamp } => {
                buf.push(3u8);
                buf.extend_from_slice(&timestamp.to_be_bytes());
            }
            Packet::Pong { timestamp } => {
                buf.push(4u8);
                buf.extend_from_slice(&timestamp.to_be_bytes());
            }
            Packet::ServerFrame {
                seq,
                data,
                compressed,
            } => {
                buf.push(5u8);
                buf.extend_from_slice(&seq.to_be_bytes());
                buf.push(if *compressed { 1u8 } else { 0u8 });
                buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
                buf.extend_from_slice(data);
            }
        }
        buf
    }

    pub fn deserialize(mut src: &[u8]) -> io::Result<Self> {
        if src.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Empty payload for Packet deserialization",
            ));
        }

        let tag = src[0];
        src = &src[1..];

        match tag {
            1 => {
                if src.len() < 4 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "ClientInput len missing"));
                }
                let len = u32::from_be_bytes(src[..4].try_into().unwrap()) as usize;
                src = &src[4..];
                if src.len() < len {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "ClientInput data missing"));
                }
                Ok(Packet::ClientInput {
                    data: src[..len].to_vec(),
                })
            }
            2 => {
                if src.len() < 4 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "ClientResize dimensions missing"));
                }
                let rows = u16::from_be_bytes(src[..2].try_into().unwrap());
                let cols = u16::from_be_bytes(src[2..4].try_into().unwrap());
                Ok(Packet::ClientResize { rows, cols })
            }
            3 => {
                if src.len() < 8 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Ping timestamp missing"));
                }
                let timestamp = u64::from_be_bytes(src[..8].try_into().unwrap());
                Ok(Packet::Ping { timestamp })
            }
            4 => {
                if src.len() < 8 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Pong timestamp missing"));
                }
                let timestamp = u64::from_be_bytes(src[..8].try_into().unwrap());
                Ok(Packet::Pong { timestamp })
            }
            5 => {
                if src.len() < 13 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "ServerFrame header missing"));
                }
                let seq = u64::from_be_bytes(src[..8].try_into().unwrap());
                let compressed = src[8] != 0;
                let len = u32::from_be_bytes(src[9..13].try_into().unwrap()) as usize;
                src = &src[13..];
                if src.len() < len {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "ServerFrame data missing"));
                }
                Ok(Packet::ServerFrame {
                    seq,
                    data: src[..len].to_vec(),
                    compressed,
                })
            }
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown packet tag: {}", other),
            )),
        }
    }
}

/// Length-prefixed framing codec: [4-byte big endian length][manual packet payload]
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
            src.reserve((4 + length) - src.len());
            return Ok(None);
        }

        src.advance(4);
        let payload = src.split_to(length);

        Packet::deserialize(&payload).map(Some)
    }
}

impl Encoder<Packet> for PacketCodec {
    type Error = io::Error;

    fn encode(&mut self, item: Packet, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let serialized = item.serialize();
        let len = serialized.len() as u32;
        dst.reserve(4 + serialized.len());
        dst.put_u32(len);
        dst.put_slice(&serialized);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_encode_decode_roundtrip() {
        let packets = vec![
            Packet::ClientInput { data: b"hello world".to_vec() },
            Packet::ClientResize { rows: 40, cols: 120 },
            Packet::Ping { timestamp: 123456789 },
            Packet::Pong { timestamp: 987654321 },
            Packet::ServerFrame {
                seq: 42,
                data: b"frame payload".to_vec(),
                compressed: true,
            },
        ];

        for pkt in packets {
            let serialized = pkt.serialize();
            let deserialized = Packet::deserialize(&serialized).expect("Deserialization failed");
            assert_eq!(pkt, deserialized);
        }
    }
}
