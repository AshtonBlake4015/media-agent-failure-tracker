use crate::infrai_client::{
    CaptureException, ExceptionPayload, FailureContext, InfraiClient, InfraiError,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Ingesting,
    Processing,
    Delivering,
    Delivered,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ingesting => "ingestion",
            Self::Processing => "processing",
            Self::Delivering => "delivery",
            Self::Delivered => "delivered",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepResult {
    Completed,
    Failed,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum WorkflowError {
    #[error("{stage} failed; later media stages were skipped")]
    StageFailed { stage: &'static str },
    #[error("invalid transition from {stage}")]
    InvalidTransition { stage: &'static str },
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
    #[error("failure capture was rejected for asset {asset_id}: {source}")]
    Tracking {
        asset_id: String,
        #[source]
        source: InfraiError,
    },
}

#[derive(Debug)]
pub struct MediaAsset {
    pub asset_id: String,
    pub creator_id: String,
    pub source_name: String,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RunOutcome {
    Delivered { asset_id: String },
    FailureCaptured { asset_id: String, stage: Stage },
}

pub fn advance(stage: Stage, result: StepResult) -> Result<Stage, WorkflowError> {
    if result == StepResult::Failed {
        return Err(WorkflowError::StageFailed {
            stage: stage.as_str(),
        });
    }

    match stage {
        Stage::Ingesting => Ok(Stage::Processing),
        Stage::Processing => Ok(Stage::Delivering),
        Stage::Delivering => Ok(Stage::Delivered),
        Stage::Delivered => Err(WorkflowError::InvalidTransition {
            stage: stage.as_str(),
        }),
    }
}

pub async fn run_asset(
    client: Option<&InfraiClient>,
    asset: MediaAsset,
    fail_at: Option<Stage>,
) -> Result<RunOutcome, ServiceError> {
    let mut stage = Stage::Ingesting;

    while stage != Stage::Delivered {
        let result = if fail_at == Some(stage) {
            StepResult::Failed
        } else {
            StepResult::Completed
        };

        match advance(stage, result) {
            Ok(next) => stage = next,
            Err(error) => {
                let client = client.expect("a tracking client is required for a failure run");
                let stage_name = stage.as_str();
                let detail = format!(
                    "asset {} ({}) stopped during {}",
                    asset.asset_id, asset.source_name, stage_name
                );
                let payload = CaptureException {
                    title: format!("media agent {} failure", stage_name),
                    message: detail.clone(),
                    exception: ExceptionPayload {
                        kind: "MediaWorkflowError",
                        value: &detail,
                    },
                    level: "error",
                    fingerprint: ["media-agent", stage_name],
                    context: FailureContext {
                        asset_id: &asset.asset_id,
                        creator_id: &asset.creator_id,
                        stage: stage_name,
                    },
                    service: "media-agent",
                    idempotency_key: format!(
                        "media-agent:{}:{}:failure",
                        asset.asset_id, stage_name
                    ),
                };
                client
                    .capture(&payload)
                    .await
                    .map_err(|source| ServiceError::Tracking {
                        asset_id: asset.asset_id.clone(),
                        source,
                    })?;

                let WorkflowError::StageFailed { .. } = error else {
                    return Err(error.into());
                };
                return Ok(RunOutcome::FailureCaptured {
                    asset_id: asset.asset_id,
                    stage,
                });
            }
        }
    }

    Ok(RunOutcome::Delivered {
        asset_id: asset.asset_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processing_failure_blocks_creator_delivery() {
        let error = advance(Stage::Processing, StepResult::Failed).unwrap_err();

        assert_eq!(
            error,
            WorkflowError::StageFailed {
                stage: "processing"
            }
        );
        assert_ne!(Stage::Processing, Stage::Delivering);
    }
}
