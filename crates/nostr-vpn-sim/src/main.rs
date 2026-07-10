use nostr_vpn_sim::{SimulationConfig, run_simulation};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report = run_simulation(SimulationConfig::default()).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
