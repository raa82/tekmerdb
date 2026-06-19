// tekmerdb-maint — maintenance utilities for TekmerDB
//
// Commands:
//   check-crb <crb_path> <parquet_path>
//       Scans the CRB for its last seq_id and checks whether that id is
//       present in the Parquet file.
//       Exit 0 — flushed, safe to rotate.
//       Exit 1 — not yet flushed, skip.
//       Exit 2 — error.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("check-crb") => {
            let crb_path = args.get(2).map(|s| s.as_str()).unwrap_or_else(|| {
                eprintln!("usage: tekmerdb-maint check-crb <crb_path> <parquet_path>");
                std::process::exit(2);
            });
            let parquet_path = args.get(3).map(|s| s.as_str()).unwrap_or_else(|| {
                eprintln!("usage: tekmerdb-maint check-crb <crb_path> <parquet_path>");
                std::process::exit(2);
            });

            match check_crb_flushed(crb_path, parquet_path) {
                Ok(true)  => { println!("flushed"); std::process::exit(0); }
                Ok(false) => { println!("not ready"); std::process::exit(1); }
                Err(e)    => { eprintln!("error: {}", e); std::process::exit(2); }
            }
        }
        _ => {
            eprintln!("usage: tekmerdb-maint check-crb <crb_path> <parquet_path>");
            std::process::exit(2);
        }
    }
}

fn check_crb_flushed(crb_path: &str, parquet_path: &str) -> anyhow::Result<bool> {
    let crb_max = max_seq_in_crb(crb_path)?;
    let crb_max = match crb_max {
        None    => return Ok(false),
        Some(m) => m,
    };

    let parquet_max = max_seq_in_parquet(parquet_path)?;
    let parquet_max = match parquet_max {
        None    => return Ok(false),
        Some(m) => m,
    };

    Ok(crb_max <= parquet_max)
}

// Scans only the 12-byte record headers (8 seq_id + 4 length), skips PFO bytes.
fn max_seq_in_crb(path: &str) -> anyhow::Result<Option<u64>> {
    if !Path::new(path).exists() {
        return Ok(None);
    }

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut max_seq: Option<u64> = None;

    loop {
        let mut hdr = [0u8; 12];
        match reader.read_exact(&mut hdr) {
            Ok(_)  => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }

        let seq_id = u64::from_le_bytes(hdr[..8].try_into().unwrap());
        let length = u32::from_le_bytes(hdr[8..12].try_into().unwrap()) as i64;

        reader.seek(SeekFrom::Current(length))?;

        max_seq = Some(max_seq.map_or(seq_id, |m: u64| m.max(seq_id)));
    }

    Ok(max_seq)
}

// Reads the seq_id column and returns the max value found.
fn max_seq_in_parquet(path: &str) -> anyhow::Result<Option<u64>> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use arrow::array::{Array, UInt64Array};

    if !Path::new(path).exists() {
        return Ok(None);
    }

    let file    = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader  = builder.with_batch_size(8192).build()?;

    let mut max_seq: Option<u64> = None;

    for batch in reader {
        let batch = batch?;
        if let Some(col) = batch.column_by_name("seq_id") {
            if let Some(arr) = col.as_any().downcast_ref::<UInt64Array>() {
                for i in 0..arr.len() {
                    if arr.is_valid(i) {
                        let v = arr.value(i);
                        max_seq = Some(max_seq.map_or(v, |m: u64| m.max(v)));
                    }
                }
            }
        }
    }

    Ok(max_seq)
}
