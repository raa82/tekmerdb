use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;
use anyhow::Result;

pub struct Fingerprinter {
    session: Session,
    tokenizer: Tokenizer,
}

impl Fingerprinter {
    pub fn new(model_path: &str, tokenizer_path: &str) -> Result<Self> {
        let session = Session::builder()?
            .commit_from_file(model_path)?;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!(e))?;

        Ok(Fingerprinter { session, tokenizer })
    }

    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let encoding = self.tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!(e))?;

        let ids: Vec<i64> = encoding.get_ids()
            .iter().map(|x| *x as i64).collect();

        let mask: Vec<i64> = encoding.get_attention_mask()
            .iter().map(|x| *x as i64).collect();

        let len = ids.len();

        // token_type_ids — all zeros for single sentence inference
        let token_type_ids: Vec<i64> = vec![0i64; len];

        let input_ids       = Tensor::<i64>::from_array(([1, len], ids))?;
        let attention_mask  = Tensor::<i64>::from_array(([1, len], mask))?;
        let token_type_ids  = Tensor::<i64>::from_array(([1, len], token_type_ids))?;

        let outputs = self.session.run(ort::inputs![
            input_ids,
            attention_mask,
            token_type_ids
        ])?;

        let (shape, data) = outputs[0].try_extract_tensor::<f32>()?;
        let seq_len = shape[1] as usize;
        let hidden  = shape[2] as usize;

        // mean pool across token dimension
        let mut pooled = vec![0.0f32; hidden];
        for i in 0..seq_len {
            for j in 0..hidden {
                pooled[j] += data[i * hidden + j];
            }
        }
        for v in pooled.iter_mut() {
            *v /= seq_len as f32;
        }

        Ok(pooled)
    }
}