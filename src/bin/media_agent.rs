use media_agent_failure_tracker::{
    infrai_client::InfraiClient,
    media_loop::{run_asset, MediaAsset, RunOutcome, Stage},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fail_at = match std::env::args().nth(1).as_deref() {
        Some("--simulate-processing-failure") => Some(Stage::Processing),
        Some(argument) => return Err(format!("unknown argument: {argument}").into()),
        None => None,
    };
    let client = fail_at.map(|_| InfraiClient::from_env()).transpose()?;
    let asset = MediaAsset {
        asset_id: "asset-demo-1042".to_owned(),
        creator_id: "creator-demo-7".to_owned(),
        source_name: "launch-cut.mov".to_owned(),
    };

    match run_asset(client.as_ref(), asset, fail_at).await? {
        RunOutcome::Delivered { asset_id } => {
            println!("delivered {asset_id} to the creator");
        }
        RunOutcome::FailureCaptured { asset_id, stage } => {
            println!("captured {stage:?} failure for {asset_id}; delivery skipped");
        }
    }
    Ok(())
}
