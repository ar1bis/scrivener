use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;

pub struct AudioModelClient {
    http: Client,
    endpoint: String,
    api_key: Option<String>,
}

impl AudioModelClient {
    pub fn new(endpoint: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            http: Client::new(),
            endpoint: endpoint.into(),
            api_key,
        }
    }

    /// Send raw PCM bytes to the audio model server via OpenAI-compatible Chat API.
    /// The server is expected to run a model like Voxtral.
    pub async fn transcribe(&self, pcm: &[u8]) -> Result<String> {
        let audio_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, pcm);

        let body = json!({
            "model": "voxtral",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_audio",
                            "input_audio": {
                                "format": "pcm",
                                "data": audio_b64
                            }
                        }
                    ]
                }
            ]
        });

        let mut req = self.http.post(&self.endpoint).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await.context("failed to send request to audio model")?;
        let json_resp: serde_json::Value = resp.json().await.context("failed to parse response")?;

        // OpenAI chat completions response shape
        let text = json_resp
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        Ok(text)
    }
}
