use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;
use anyhow::Result;

#[allow(dead_code)]
#[derive(Debug)]
pub enum NliLabel {
    Contradiction,
    Entailment,
    Neutral,
}

pub struct NliClassifier {
    session: Session,
    tokenizer: Tokenizer,
}

impl NliClassifier {
    pub fn new(model_path: &str, tokenizer_path: &str) -> Result<Self> {
        let session = Session::builder()?
            .commit_from_file(model_path)?;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!(e))?;

        Ok(NliClassifier { session, tokenizer })
    }

    fn run_inference(&mut self, premise: &str, hypothesis: &str) -> Result<(f32, f32, f32)> {
        let encoding = self.tokenizer
            .encode((premise, hypothesis), true)
            .map_err(|e| anyhow::anyhow!(e))?;

        let ids: Vec<i64> = encoding.get_ids()
            .iter().map(|x| *x as i64).collect();

        let mask: Vec<i64> = encoding.get_attention_mask()
            .iter().map(|x| *x as i64).collect();

        let len = ids.len();

        let input_ids      = Tensor::<i64>::from_array(([1, len], ids))?;
        let attention_mask = Tensor::<i64>::from_array(([1, len], mask))?;

        let outputs = self.session.run(ort::inputs![
            input_ids,
            attention_mask
        ])?;

        let (_, data) = outputs[0].try_extract_tensor::<f32>()?;

        Ok((data[0], data[1], data[2]))
    }

    // returns softmax probabilities (contradiction, entailment, neutral)
    pub fn classify_probs(&mut self, premise: &str, hypothesis: &str) -> Result<(f32, f32, f32)> {
        let (c, e, n) = self.run_inference(premise, hypothesis)?;
        let exp_c = c.exp();
        let exp_e = e.exp();
        let exp_n = n.exp();
        let sum = exp_c + exp_e + exp_n;
        Ok((exp_c / sum, exp_e / sum, exp_n / sum))
    }

    // kept for any direct label usage
    #[allow(dead_code)]
    pub fn classify(&mut self, premise: &str, hypothesis: &str) -> Result<NliLabel> {
        let (prob_c, prob_e, prob_n) = self.classify_probs(premise, hypothesis)?;
        if prob_c > prob_e && prob_c > prob_n {
            Ok(NliLabel::Contradiction)
        } else if prob_e > prob_c && prob_e > prob_n {
            Ok(NliLabel::Entailment)
        } else {
            Ok(NliLabel::Neutral)
        }
    }
}