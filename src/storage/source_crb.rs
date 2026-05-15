use std::fs::{File, OpenOptions};
use std::io::{Write, BufWriter, BufReader, Read};
use serde::{Deserialize, Serialize};
use crate::engine::pfo::Source;

// SourceRecord wraps Source with its name for persistence
// Name is not on Source struct — stored here so the name_index can be rebuilt on startup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecord {
    pub name: String,
    pub source: Source,
    pub seq_id: u64,
}

pub struct SourceCrb {
    writer: BufWriter<File>,
    path: String,
}

impl SourceCrb {
    pub fn new(path: &str) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        Ok(SourceCrb {
            writer: BufWriter::new(file),
            path: path.to_string(),
        })
    }

    pub fn write(&mut self, record: &SourceRecord) -> std::io::Result<()> {
        let encoded = bincode::serialize(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let len = encoded.len() as u32;

        // header: [ 8 bytes seq_id ][ 4 bytes length ] — same format as PFO CRB
        self.writer.write_all(&record.seq_id.to_le_bytes())?;
        self.writer.write_all(&len.to_le_bytes())?;
        self.writer.write_all(&encoded)?;
        self.writer.flush()?;
        self.writer.get_ref().sync_all()?;
        Ok(())
    }

    // returns (records, max_seq_id) — same signature as PFO CRB replay
    pub fn replay(&self) -> std::io::Result<(Vec<SourceRecord>, u64)> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((vec![], 0));
            }
            Err(e) => return Err(e),
        };

        let mut reader = BufReader::new(file);
        let mut records = Vec::new();
        let mut max_seq: u64 = 0;

        loop {
            let mut seq_bytes = [0u8; 8];
            match reader.read_exact(&mut seq_bytes) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            let seq_id = u64::from_le_bytes(seq_bytes);

            let mut len_bytes = [0u8; 4];
            match reader.read_exact(&mut len_bytes) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            let len = u32::from_le_bytes(len_bytes) as usize;

            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf)?;

            let record: SourceRecord = bincode::deserialize(&buf)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            if seq_id > max_seq {
                max_seq = seq_id;
            }

            records.push(record);
        }

        Ok((records, max_seq))
    }
}