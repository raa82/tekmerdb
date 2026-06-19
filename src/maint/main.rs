// tekmerdb-maint — maintenance utilities for TekmerDB
//
// Commands:
//   check-crb  <crb_path> <parquet_path>
//       Exit 0 if the last seq_id in the CRB is present in the Parquet file.
//       Exit 1 if not yet flushed. Exit 2 on error.
//
//   rotate-crb <crb_path> <parquet_path>
//       Same flush check, then renames the CRB to a zero-padded numbered
//       archive: crb.00001.bin, crb.00002.bin, … (counter increments per file).
//       Exit 0 rotated. Exit 1 not ready (skipped). Exit 2 on error.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("check-crb") => {
            let (crb_path, parquet_path) = parse_two_args(&args, "check-crb");
            match check_crb_flushed(crb_path, parquet_path) {
                Ok(true)  => { println!("flushed"); std::process::exit(0); }
                Ok(false) => { println!("not ready"); std::process::exit(1); }
                Err(e)    => { eprintln!("error: {}", e); std::process::exit(2); }
            }
        }
        Some("rotate-crb") => {
            let (crb_path, parquet_path) = parse_two_args(&args, "rotate-crb");
            match rotate_crb(crb_path, parquet_path) {
                Ok(Some(dest)) => { println!("rotated to {}", dest); std::process::exit(0); }
                Ok(None)       => { println!("not ready, skipping"); std::process::exit(1); }
                Err(e)         => { eprintln!("error: {}", e); std::process::exit(2); }
            }
        }
        _ => {
            eprintln!("usage: tekmerdb-maint <check-crb|rotate-crb> <crb_path> <parquet_path>");
            std::process::exit(2);
        }
    }
}

fn parse_two_args<'a>(args: &'a [String], cmd: &str) -> (&'a str, &'a str) {
    let a = args.get(2).map(|s| s.as_str()).unwrap_or_else(|| {
        eprintln!("usage: tekmerdb-maint {} <crb_path> <parquet_path>", cmd);
        std::process::exit(2);
    });
    let b = args.get(3).map(|s| s.as_str()).unwrap_or_else(|| {
        eprintln!("usage: tekmerdb-maint {} <crb_path> <parquet_path>", cmd);
        std::process::exit(2);
    });
    (a, b)
}

// Renames <crb_path> to <stem>.<counter>.bin where counter is the next
// available zero-padded 5-digit number in the same directory.
fn rotate_crb(crb_path: &str, parquet_path: &str) -> anyhow::Result<Option<String>> {
    if !check_crb_flushed(crb_path, parquet_path)? {
        return Ok(None);
    }

    let path = Path::new(crb_path);
    let dir  = path.parent().unwrap_or(Path::new("."));
    let filename = path.file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid crb path: {}", crb_path))?
        .to_string_lossy();

    // stem: "crb" from "crb.bin", "source_crb" from "source_crb.bin"
    let stem = filename.strip_suffix(".bin")
        .ok_or_else(|| anyhow::anyhow!("expected .bin file: {}", filename))?;

    let counter = next_counter(dir, stem)?;
    let dest = dir.join(format!("{}.{:06}.bin", stem, counter));

    std::fs::rename(crb_path, &dest)?;

    Ok(Some(dest.to_string_lossy().into_owned()))
}

// Scans the directory for files matching <stem>.<digits>.bin and returns
// the next counter (max existing + 1, starting at 1 if none exist).
fn next_counter(dir: &Path, stem: &str) -> anyhow::Result<u32> {
    let prefix = format!("{}.", stem);
    let mut max: u32 = 0;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name  = entry.file_name();
        let name  = name.to_string_lossy();

        if let Some(rest) = name.strip_prefix(&prefix) {
            if let Some(num_str) = rest.strip_suffix(".bin") {
                if let Ok(n) = num_str.parse::<u32>() {
                    if n > max { max = n; }
                }
            }
        }
    }

    Ok(max + 1)
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
