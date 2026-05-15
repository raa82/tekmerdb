use std::fs;
use std::path::Path;
use std::sync::Arc;
use arrow::array::{StringArray, Float32Array, UInt32Array, UInt64Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;
use crate::engine::pfo::{Pfo, Domain};
use uuid::Uuid;
use chrono::DateTime;

pub struct ColdTier {
    path: String,
}

impl ColdTier {
    pub fn new(path: &str) -> anyhow::Result<Self> {
        fs::create_dir_all(Path::new(path).parent().unwrap_or(Path::new(".")))?;
        Ok(ColdTier {
            path: path.to_string(),
        })
    }

    fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id",                  DataType::Utf8,    false),
            Field::new("seq_id",              DataType::UInt64,  false),
            Field::new("claim_text",          DataType::Utf8,    false),
            Field::new("confidence",          DataType::Float32, false),
            Field::new("corroboration_count", DataType::UInt32,  false),
            Field::new("domain",              DataType::Utf8,    false),
            Field::new("created_at",          DataType::Utf8,    false),
            Field::new("source",              DataType::Utf8,    false),  // original string
            Field::new("source_id",           DataType::Utf8,    false),  // resolved UUID
            Field::new("conflict_refs",       DataType::Utf8,    false),
        ]))
    }

    pub fn flush(&self, pfos: &[Pfo]) -> anyhow::Result<()> {
        if pfos.is_empty() {
            return Ok(());
        }

        let schema = Self::schema();

        let ids: StringArray = pfos.iter()
            .map(|p| Some(p.id.to_string()))
            .collect();

        let seq_ids: UInt64Array = pfos.iter()
            .map(|p| Some(p.seq_id))
            .collect();

        let claims: StringArray = pfos.iter()
            .map(|p| Some(p.claim_text.as_str()))
            .collect();

        let confidences: Float32Array = pfos.iter()
            .map(|p| Some(p.confidence))
            .collect();

        let corroborations: UInt32Array = pfos.iter()
            .map(|p| Some(p.corroboration_count))
            .collect();

        let domains: StringArray = pfos.iter()
            .map(|p| Some(format!("{:?}", p.domain)))
            .collect();

        let created_ats: StringArray = pfos.iter()
            .map(|p| Some(p.created_at.to_rfc3339()))
            .collect();

        let sources: StringArray = pfos.iter()
            .map(|p| Some(p.source.as_str()))
            .collect();

        let source_ids: StringArray = pfos.iter()
            .map(|p| Some(p.source_id.to_string()))
            .collect();

        let conflict_refs: StringArray = pfos.iter()
            .map(|p| Some(serde_json::to_string(&p.conflict_refs).unwrap_or_default()))
            .collect();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(ids),
                Arc::new(seq_ids),
                Arc::new(claims),
                Arc::new(confidences),
                Arc::new(corroborations),
                Arc::new(domains),
                Arc::new(created_ats),
                Arc::new(sources),
                Arc::new(source_ids),
                Arc::new(conflict_refs),
            ],
        )?;

        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;

        let props = WriterProperties::builder().build();
        let mut writer = ArrowWriter::try_new(file, schema, Some(props))?;
        writer.write(&batch)?;
        writer.close()?;

        println!("[cold tier] flushed {} PFO(s) to parquet", pfos.len());
        Ok(())
    }

    pub fn read(&self) -> anyhow::Result<(Vec<Pfo>, u64)> {
        if !Path::new(&self.path).exists() {
            return Ok((vec![], 0));
        }

        let file = fs::File::open(&self.path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        let mut reader = builder.build()?;

        let mut pfos = Vec::new();
        let mut max_seq: u64 = 0;

        while let Some(batch) = reader.next() {
            let batch = batch?;

            let ids        = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
            let seq_ids    = batch.column(1).as_any().downcast_ref::<UInt64Array>().unwrap();
            let claims     = batch.column(2).as_any().downcast_ref::<StringArray>().unwrap();
            let confs      = batch.column(3).as_any().downcast_ref::<Float32Array>().unwrap();
            let corr       = batch.column(4).as_any().downcast_ref::<UInt32Array>().unwrap();
            let domains    = batch.column(5).as_any().downcast_ref::<StringArray>().unwrap();
            let created    = batch.column(6).as_any().downcast_ref::<StringArray>().unwrap();
            let sources    = batch.column(7).as_any().downcast_ref::<StringArray>().unwrap();
            let source_ids = batch.column(8).as_any().downcast_ref::<StringArray>().unwrap();
            let confrefs   = batch.column(9).as_any().downcast_ref::<StringArray>().unwrap();

            for i in 0..batch.num_rows() {
                let seq_id = seq_ids.value(i);
                if seq_id > max_seq { max_seq = seq_id; }

                let conflict_refs_vec: Vec<Uuid> = serde_json::from_str(confrefs.value(i))
                    .unwrap_or_default();

                let pfo = Pfo {
                    id: Uuid::parse_str(ids.value(i)).unwrap_or_else(|_| Uuid::new_v4()),
                    seq_id,
                    claim_text: claims.value(i).to_string(),
                    semantic_fingerprint: vec![],
                    confidence: confs.value(i),
                    corroboration_count: corr.value(i),
                    domain: parse_domain(domains.value(i)),
                    created_at: DateTime::parse_from_rfc3339(created.value(i))
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    last_corroborated: None,
                    source: sources.value(i).to_string(),
                    source_id: Uuid::parse_str(source_ids.value(i))
                        .unwrap_or_else(|_| Uuid::nil()),
                    conflict_refs: conflict_refs_vec,
                    dirty: false,
                };

                pfos.push(pfo);
            }
        }

        Ok((pfos, max_seq))
    }
}

fn parse_domain(s: &str) -> Domain {
    match s {
        "Healthcare"             => Domain::Healthcare,
        "LawEnforcement"         => Domain::LawEnforcement,
        "CriticalInfrastructure" => Domain::CriticalInfrastructure,
        "Employment"             => Domain::Employment,
        "Education"              => Domain::Education,
        "Migration"              => Domain::Migration,
        "LegalInterpretation"    => Domain::LegalInterpretation,
        "Finance"                => Domain::Finance,
        _                        => Domain::General,
    }
}