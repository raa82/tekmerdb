use std::fs::{File, OpenOptions};
use std::io::{Write, BufWriter, BufReader, Read};
use crate::engine::pfo::Pfo;

pub struct Crb {
    writer: BufWriter<File>,
    path: String,
}

impl Crb {
    pub fn new(path: &str) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        Ok(Crb {
            writer: BufWriter::new(file),
            path: path.to_string(),
        })
    }

    pub fn write(&mut self, pfo: &Pfo) -> std::io::Result<()> {
        let encoded = bincode::serialize(pfo)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let len = encoded.len() as u32;

        // header: [ 8 bytes seq_id ][ 4 bytes length ]
        self.writer.write_all(&pfo.seq_id.to_le_bytes())?;
        self.writer.write_all(&len.to_le_bytes())?;
        self.writer.write_all(&encoded)?;
        self.writer.flush()?;
        self.writer.get_ref().sync_all()?;
        Ok(())
    }

    // returns (pfos, max_seq_id) so engine can restore the counter
    pub fn replay(&self) -> std::io::Result<(Vec<Pfo>, u64)> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((vec![], 0));
            }
            Err(e) => return Err(e),
        };

        let mut reader = BufReader::new(file);
        let mut pfos = Vec::new();
        let mut max_seq: u64 = 0;

        loop {
            // read 8-byte seq_id header
            let mut seq_bytes = [0u8; 8];
            match reader.read_exact(&mut seq_bytes) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            let seq_id = u64::from_le_bytes(seq_bytes);

            // read 4-byte length header
            let mut len_bytes = [0u8; 4];
            match reader.read_exact(&mut len_bytes) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            let len = u32::from_le_bytes(len_bytes) as usize;

            // read PFO bytes
            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf)?;

            let pfo: Pfo = bincode::deserialize(&buf)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            if seq_id > max_seq {
                max_seq = seq_id;
            }

            pfos.push(pfo);
        }

        Ok((pfos, max_seq))
    }
}