use crate::log_info;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use arrow::array::{StringArray, Float32Array, UInt32Array, UInt64Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;
use crate::engine::pfo::Source;
use crate::storage::source_crb::SourceRecord;
use uuid::Uuid;

pub struct SourceColdTier {
    path: String,
}

impl SourceColdTier {
    pub fn new(path: &str) -> anyhow::Result<Self> {
        fs::create_dir_all(Path::new(path).parent().unwrap_or(Path::new(".")))?;
        Ok(SourceColdTier {
            path: path.to_string(),
        })
    }

    fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("source_id",             DataType::Utf8,    false),
            Field::new("name",                  DataType::Utf8,    false),
            Field::new("seq_id",                DataType::UInt64,  false),
            Field::new("source_weight",         DataType::Float32, false),
            Field::new("effective_weight",      DataType::Float32, false),
            Field::new("corroboration_count",   DataType::UInt32,  false),
            Field::new("conflict_trigger_count",DataType::UInt32,  false),
        ]))
    }

    // overwrite on every flush — hot tier (SourceRegistry) is source of truth
    pub fn flush(&self, records: &[SourceRecord]) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let schema = Self::schema();

        let source_ids: StringArray = records.iter()
            .map(|r| Some(r.source.source_id.to_string()))
            .collect();

        let names: StringArray = records.iter()
            .map(|r| Some(r.name.as_str()))
            .collect();

        let seq_ids: UInt64Array = records.iter()
            .map(|r| Some(r.seq_id))
            .collect();

        let source_weights: Float32Array = records.iter()
            .map(|r| Some(r.source.source_weight))
            .collect();

        let effective_weights: Float32Array = records.iter()
            .map(|r| Some(r.source.effective_weight))
            .collect();

        let corroboration_counts: UInt32Array = records.iter()
            .map(|r| Some(r.source.corroboration_count))
            .collect();

        let conflict_trigger_counts: UInt32Array = records.iter()
            .map(|r| Some(r.source.conflict_trigger_count))
            .collect();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(source_ids),
                Arc::new(names),
                Arc::new(seq_ids),
                Arc::new(source_weights),
                Arc::new(effective_weights),
                Arc::new(corroboration_counts),
                Arc::new(conflict_trigger_counts),
            ],
        )?;

        // overwrite — same pattern as cold.rs
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;

        let props = WriterProperties::builder().build();
        let mut writer = ArrowWriter::try_new(file, schema, Some(props))?;
        writer.write(&batch)?;
        writer.close()?;

        log_info!("[source_cold] flushed {} source(s) to parquet", records.len());
        Ok(())
    }

    // returns (records, max_seq_id) — same signature as ColdTier::read
    pub fn read(&self) -> anyhow::Result<(Vec<SourceRecord>, u64)> {
        if !Path::new(&self.path).exists() {
            return Ok((vec![], 0));
        }

        let file = fs::File::open(&self.path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        let mut reader = builder.build()?;

        let mut records = Vec::new();
        let mut max_seq: u64 = 0;

        while let Some(batch) = reader.next() {
            let batch = batch?;

            let source_ids  = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
            let names       = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
            let seq_ids     = batch.column(2).as_any().downcast_ref::<UInt64Array>().unwrap();
            let sw          = batch.column(3).as_any().downcast_ref::<Float32Array>().unwrap();
            let ew          = batch.column(4).as_any().downcast_ref::<Float32Array>().unwrap();
            let cc          = batch.column(5).as_any().downcast_ref::<UInt32Array>().unwrap();
            let ctc         = batch.column(6).as_any().downcast_ref::<UInt32Array>().unwrap();

            for i in 0..batch.num_rows() {
                let seq_id = seq_ids.value(i);
                if seq_id > max_seq { max_seq = seq_id; }

                let record = SourceRecord {
                    name: names.value(i).to_string(),
                    seq_id,
                    source: Source {
                        source_id:             Uuid::parse_str(source_ids.value(i)).unwrap_or_else(|_| Uuid::new_v4()),
                        source_weight:         sw.value(i),
                        effective_weight:      ew.value(i),
                        corroboration_count:   cc.value(i),
                        conflict_trigger_count: ctc.value(i),
                    },
                };

                records.push(record);
            }
        }

        Ok((records, max_seq))
    }
}