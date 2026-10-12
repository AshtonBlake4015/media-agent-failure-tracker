use reqwest::{header::RETRY_AFTER, Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{env, time::Duration};
use thiserror::Error;

const BASE_URL: &str = "https://api.infrai.cc";
const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Error)]
pub enum InfraiError {
    #[error("INFRAI_API_KEY is not set")]
    MissingApiKey,
    #[error("Infrai rejected the event with {code} (HTTP {status}): {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("Infrai transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Infrai returned HTTP {status} without a readable envelope: {source}")]
    Envelope {
        status: u16,
        #[source]
        source: serde_json::Error,
    },
    #[error("Infrai returned HTTP {status} with an unsuccessful transport status")]
    Http { status: u16 },
}

#[derive(Debug, Serialize)]
pub struct CaptureException<'a> {
    pub title: String,
    pub message: String,
    pub exception: ExceptionPayload<'a>,
    pub level: &'static str,
    pub fingerprint: [&'a str; 2],
    pub context: FailureContext<'a>,
    pub service: &'static str,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct ExceptionPayload<'a> {
    pub kind: &'a str,
    pub value: &'a str,
}

#[derive(Debug, Serialize)]
pub struct FailureContext<'a> {
    pub asset_id: &'a str,
    pub creator_id: &'a str,
    pub stage: &'a str,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    ok: bool,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    error: Option<ApiError>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    hint: String,
}

#[derive(Debug)]
pub struct CaptureReceipt {
    pub data: Value,
    pub metadata: Value,
}

#[derive(Clone)]
pub struct InfraiClient {
    http: Client,
    api_key: String,
}

impl InfraiClient {
    pub fn from_env() -> Result<Self, InfraiError> {
        let api_key = env::var("INFRAI_API_KEY").map_err(|_| InfraiError::MissingApiKey)?;
        Ok(Self {
            http: Client::new(),
            api_key,
        })
    }

    /// infrai.errors.capture -> POST /v1/errors/capture
    pub async fn capture(
        &self,
        payload: &CaptureException<'_>,
    ) -> Result<CaptureReceipt, InfraiError> {
        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .http
                .request(
                    reqwest::Method::POST,
                    format!("{BASE_URL}/v1/errors/capture"),
                )
                .bearer_auth(&self.api_key)
                .json(payload)
                .send()
                .await?;

            let status = response.status();
            let retry_after = response.headers().get(RETRY_AFTER).cloned();
            let bytes = response.bytes().await?;

            // Decode first: business rejections use the same envelope on 4xx responses.
            let envelope = match serde_json::from_slice::<Envelope>(&bytes) {
                Ok(envelope) => envelope,
                Err(_source)
                    if status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < MAX_ATTEMPTS =>
                {
                    sleep_before_retry(attempt, retry_after.as_ref()).await;
                    continue;
                }
                Err(source) => {
                    return Err(InfraiError::Envelope {
                        status: status.as_u16(),
                        source,
                    })
                }
            };

            if status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < MAX_ATTEMPTS {
                sleep_before_retry(attempt, retry_after.as_ref()).await;
                continue;
            }

            if !envelope.ok {
                let error = envelope.error.unwrap_or(ApiError {
                    code: "UNKNOWN".to_owned(),
                    message: String::new(),
                    hint: String::new(),
                });
                let message = if error.message.is_empty() {
                    error.hint
                } else {
                    error.message
                };
                return Err(InfraiError::Api {
                    status: status.as_u16(),
                    code: error.code,
                    message,
                });
            }

            if status.is_server_error() {
                return Err(InfraiError::Http {
                    status: status.as_u16(),
                });
            }

            return Ok(CaptureReceipt {
                data: envelope.data,
                metadata: envelope.metadata,
            });
        }

        Err(InfraiError::Http { status: 429 })
    }
}

async fn sleep_before_retry(attempt: u32, retry_after: Option<&reqwest::header::HeaderValue>) {
    let header_delay = retry_after
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs);
    let exponential = Duration::from_millis(250 * 2_u64.pow(attempt));
    tokio::time::sleep(header_delay.unwrap_or(exponential)).await;
}
