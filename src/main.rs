use anyhow::anyhow;
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    SimpleLogger::new()
        .env()
        .init()
        .map_err(|err| anyhow!("Error while init logger: {}", err))?;

    #[cfg(feature = "solana")]
    {
        use std::{env, str::FromStr};

        use solana_events_parser::transaction_parser::*;

        let meta = RpcClient::new("https://api.mainnet-beta.solana.com".to_string())
            .bind_transaction_instructions_logs(
                Signature::from_str(&env::args().nth(1).ok_or_else(|| {
                    anyhow!(
                    "Signatures not provided, Use first argument for provide transaction signature"
                )
                })?)
                .map_err(|err| {
                    anyhow!(
                        "Error while parsing argument as transaction signature: {}",
                        err
                    )
                })?,
            )
            .await
            .map_err(|err| anyhow!("Error while bind transaction instructions: {}", err))?;

        println!(
            "{}",
            serde_json::to_string_pretty(&meta.meta.into_iter().collect::<Vec<_>>())
                .map_err(|err| { anyhow!("Error while serialize result of binding: {}", err) })?
        );
        println!("Lamports changes: {:?}", meta.lamports_changes);
        println!("Token balance changes: {:?}", meta.token_balances_changes);
    }

    #[cfg(not(feature = "solana"))]
    println!("No action when solana feature disable");

    Ok(())
}
