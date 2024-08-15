mod db;
mod process_artists;
mod spotify_auth;

use spotify_auth::{initialize_redis_pool, set_tokens};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    initialize_redis_pool().await?;
    println!("Redis pool initialized successfully");

    set_tokens().await?;
    println!("Spotify tokens set up successfully");

    process_artists::process().await?;
    println!("Artist processing completed");

    Ok(())
}
