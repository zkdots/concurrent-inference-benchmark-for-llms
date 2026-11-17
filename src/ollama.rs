use crate::config::Args;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

const MAX_BODY: usize = 16 * 1024 * 1024;

async fn body(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err("response exceeds 16 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        return Err(format!(
            "HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    Ok(bytes)
}

#[derive(Debug, Deserialize)]
pub(crate) struct Generation {
    done: bool,
    pub(crate) eval_count: u64,
    pub(crate) eval_duration: u64,
    #[serde(default)]
    pub(crate) load_duration: u64,
}

pub(crate) async fn generate(client: &Client, a: &Args, tokens: u32) -> Result<Generation, String> {
    let request = async {
        let response = client.post(format!("{}/api/generate", a.url)).json(&json!({
            "model": a.model, "prompt": a.prompt, "stream": false,
            "keep_alive": "5m", "options": { "num_predict": tokens, "temperature": 0, "seed": 42 }
        })).send().await.map_err(|e| e.to_string())?;
        let bytes = body(response).await?;
        let g: Generation =
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid Ollama response: {e}"))?;
        if !g.done {
            return Err("incomplete generation (done=false)".into());
        }
        Ok(g)
    };
    tokio::time::timeout(a.timeout, request)
        .await
        .map_err(|_| "request timeout".to_string())?
}

pub(crate) async fn resolve_model(client: &Client, a: &mut Args) -> Result<(), String> {
    if a.model.is_none() {
        #[derive(Deserialize)]
        struct Model {
            name: String,
        }
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<Model>,
        }
        let response = client
            .get(format!("{}/api/tags", a.url))
            .send()
            .await
            .map_err(|e| format!("cannot reach Ollama: {e}"))?;
        let tags: Tags =
            serde_json::from_slice(&body(response).await?).map_err(|e| e.to_string())?;
        if tags.models.len() != 1 {
            return Err(format!(
                "specify --model; installed: {}",
                tags.models
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        a.model = Some(tags.models[0].name.clone());
    }
    Ok(())
}
