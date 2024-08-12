use chrono::{DateTime, Utc};
use tokio::{io::AsyncReadExt, net::TcpListener};
use deadpool_redis::{Config as RedisConfig, Pool, Runtime};
use once_cell::sync::OnceCell;
use redis::AsyncCommands;
use reqwest::Client as reqwest;
use serde::Deserialize;
use std::env;
use std::error::Error;
use std::time::Duration;
use dotenv::dotenv;
#[derive(Deserialize, Debug)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

 static REDIS_POOL: OnceCell<Pool> = OnceCell::new();

pub async fn initialize_redis_pool() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set");
    let config = RedisConfig::from_url(redis_url);
    let pool = config.create_pool(Some(Runtime::Tokio1)).unwrap();
    REDIS_POOL
        .set(pool)
        .expect("Failed to initialize Redis pool");
    Ok(())
}

pub async fn get_redis_pool() -> &'static Pool {
    REDIS_POOL.get().expect("Redis pool not initialized")
}

pub async fn authorize_client() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
     let pool = get_redis_pool().await;
    let app_id = env::var("SPOTIFY_CLIENT_ID").expect("SPOTIFY_CLIENT_ID must be set");
    let redirect_uri = env::var("REDIRECT_URI").expect("REDIRECT_URI must be set");
    let scopes = "playlist-read-private";

    let auth_url = format!(
        "https://accounts.spotify.com/authorize?client_id={}&response_type=code&redirect_uri={}&scope={}",
        app_id, redirect_uri, scopes
    );

    println!(
        "Please authorize the app by visiting this URL: {:?}",
        auth_url
    );
     let listener = TcpListener::bind("127.0.0.1:7979").await?;
    println!("Listening to incoming connections on localhost:7979....");

    let (mut socket, _) = listener.accept().await?;
    let mut buffer = [0; 1024];
    let bytes_read = socket.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);

    if let Some(start) = request.find("code=") {
        let code_start = start + 5;
        let code_end = request[code_start..]
            .find(&[' ', '&', '\n'][..])
            .unwrap_or_else(|| request.len() - code_start)
            + code_start;

        let code = request[code_start..code_end].to_string();

        set_key(pool, "auth_code", &code).await?;
        println!("Authorization code saved to Redis");
    } else {
        println!("Authorization code not found in the request.");
    }
    Ok(())
}

pub async fn set_tokens() -> Result<(), Box<dyn Error>> {
    if !auth_code_exists().await? {
        authorize_client().await?;
    }

    if !refresh_token_exists().await? {
        set_refresh_token().await?;
    }

    if !access_token_exists().await? || check_access_token_expired().await? {
        set_access_token().await?;
    }

    Ok(())
}

pub async fn set_refresh_token() -> Result<(), Box<dyn Error>> {
    let pool = get_redis_pool().await;
    let client = reqwest::new();

    println!("Fetching refresh token...");

    let auth_code = get_key(pool, "auth_code").await?;

    let redirect_uri = env::var("REDIRECT_URI").expect("REDIRECT_URI must be set");
    let client_id = env::var("SPOTIFY_CLIENT_ID").expect("SPOTIFY_CLIENT_ID must be set");
    let client_secret = env::var("SPOTIFY_CLIENT_SECRET").expect("SPOTIFY_CLIENT_SECRET must be set");

    let params = [
        ("grant_type", "authorization_code"),
        ("code", auth_code.trim()),
        ("redirect_uri", &redirect_uri),
        ("client_id", &client_id),
        ("client_secret", &client_secret),
    ];

    let res = client
        .post("https://accounts.spotify.com/api/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await?;

    let response_text = res.text().await?;
    println!("Spotify response: {}", response_text);

    let token_response: TokenResponse = serde_json::from_str(&response_text)?;
    if let Some(refresh_token) = token_response.refresh_token {
        set_key(pool, "refresh_token", &refresh_token).await?;
        println!("Refresh token set");
    } else {
        println!("No refresh token received");
    }

    set_key(pool, "access_token", &token_response.access_token).await?;
    set_token_expiry_time(token_response.expires_in).await?;
    println!("Access token set");

    Ok(())
}

async fn set_access_token() -> Result<(), Box<dyn Error>> {
    let pool = get_redis_pool().await;
    let client = reqwest::new();

    println!("Access token expired/not found, getting new access token");

    let refresh_token = get_key(pool, "refresh_token").await?;
    let client_id = env::var("SPOTIFY_CLIENT_ID").expect("SPOTIFY_CLIENT_ID must be set");
    let client_secret = env::var("SPOTIFY_CLIENT_SECRET").expect("SPOTIFY_CLIENT_SECRET must be set");

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.trim()),
        ("client_id", &client_id),
        ("client_secret", &client_secret),
    ];

    let res = client
        .post("https://accounts.spotify.com/api/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await?;

    let response_text = res.text().await?;
    println!("Spotify response: {}", response_text);

    let token_response: TokenResponse = serde_json::from_str(&response_text)?;
    let access_token = token_response.access_token.clone();
    let expiry = token_response.expires_in;
    set_token_expiry_time(expiry).await?;

    set_key(pool, "access_token", &access_token).await?;
    println!("New access token set");

    Ok(())
}

pub async fn set_key(pool: &Pool, key: &str, value: &str) -> Result<(), Box<dyn Error>> {
    let mut conn = pool.get().await?;
    conn.set(key, value).await?;
    println!("Key set: {}", key);
    Ok(())
}

pub async fn get_key(pool: &Pool, key: &str) -> Result<String, Box<dyn Error>> {
    let mut conn = pool.get().await?;
    let value: String = conn.get(key).await?;
    Ok(value)
}

async fn auth_code_exists() -> Result<bool, Box<dyn Error>> {
    let pool = get_redis_pool().await;
    let mut conn = pool.get().await?;
    let exists: bool = conn.exists("auth_code").await?;
    Ok(exists)
}

async fn set_token_expiry_time(expiry: u64) -> Result<(), Box<dyn Error>> {
    let pool = get_redis_pool().await;
    let expiry_time = Utc::now() + Duration::from_secs(expiry);
    let expiry_time = expiry_time.to_rfc3339();
    set_key(pool, "expiry", &expiry_time).await
}

async fn check_access_token_expired() -> Result<bool, Box<dyn std::error::Error>> {
    let pool = get_redis_pool().await;
    let expiry = get_key(pool, "expiry").await?;
    let expiry = expiry.parse::<DateTime<Utc>>()?;

    Ok(Utc::now() > expiry)
}

async fn access_token_exists() -> Result<bool, Box<dyn Error>> {
    let pool = get_redis_pool().await;
    let mut conn = pool.get().await?;
    let exists: bool = conn.exists("access_token").await?;
    Ok(exists)
}

async fn refresh_token_exists() -> Result<bool, Box<dyn Error>> {
    let pool = get_redis_pool().await;
    let mut conn = pool.get().await?;
    let exists: bool = conn.exists("refresh_token").await?;
    Ok(exists)
}


