mod db;
mod dbupdate;
mod spotify_auth;
use spotify_auth::{initialize_redis_pool, set_tokens};
use std::error::Error;
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match initialize_redis_pool().await {
        Ok(_) => println!("Redis pool initialized successfully"),
        Err(e) => eprintln!("Failed to initialize Redis pool: {}", e),
    }
    set_tokens().await?;
    dbupdate::dbupdate().await?;
    Ok(())
}

