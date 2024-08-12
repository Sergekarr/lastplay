use crate::db::{add_artist, create_table, establish_db_pool, update_artist_playcount};
use crate::spotify_auth::{get_key, get_redis_pool};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Deserialize, Debug)]
struct Response {
    artists: Value,
}

async fn fetch_artists(page: u64) -> Result<Response, Box<dyn Error>> {
    let username = env::var("LASTFM_USERNAME").expect("LASTFM_USERNAME must be set");
    let api_key = env::var("LASTFM_API_KEY").expect("LASTFM_API_KEY must be set");
    let url = format!(
        "http://ws.audioscrobbler.com/2.0/?method=library.getartists&api_key={}&limit=2000&page={}&user={}&format=json",
        api_key, page, username,
    );
    let response_text = reqwest::get(&url).await?.text().await?;
    let response: Response = serde_json::from_str(&response_text)?;
    Ok(response)
}

pub async fn dbupdate() -> Result<(), Box<dyn Error>> {
    let username = env::var("LASTFM_USERNAME").expect("LASTFM_USERNAME must be set");
    let api_key = env::var("LASTFM_API_KEY").expect("LASTFM_API_KEY must be set");
    let initial_url = format!("http://ws.audioscrobbler.com/2.0/?method=library.getartists&api_key={}&limit=2000&user={}&format=json", api_key, username);
    let initial_response_text = reqwest::get(initial_url).await?.text().await?;
    let initial_response: Response = serde_json::from_str(&initial_response_text)?;
    let current_page: u64 = initial_response.artists["@attr"]["page"]
        .as_str()
        .unwrap()
        .parse()?;
    let total_pages: u64 = initial_response.artists["@attr"]["totalPages"]
        .as_str()
        .unwrap()
        .parse()?;

    let pool = establish_db_pool()?;
    create_table(&pool)?;
    let mut existing_artists = HashMap::new();
    let conn = pool.get()?;
    let mut stmt = conn.prepare("SELECT name, playcount FROM artists")?;
    let artist_iter = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    for artist in artist_iter {
        let (name, playcount): (String, u64) = artist?;
        existing_artists.insert(name, playcount);
    }

    let existing_artists = Arc::new(RwLock::new(existing_artists));
    let mut handles = vec![];
    for page in current_page..=total_pages {
        let conn_clone = pool.clone();
        let existing_artists_clone = existing_artists.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = process_artists(page, conn_clone, existing_artists_clone).await {
                eprintln!("Error processing artists for page {}: {}", page, e);
            }
        }));
    }

    for handle in handles {
        handle.await?;
    }

    Ok(())
}

pub async fn get_seed(artist: &str) -> Result<String, Box<dyn Error>> {
    let client = Client::new();
    let pool = get_redis_pool().await;
    let access_token = get_key(pool, "access_token").await?;

    let token_arc = Arc::new(RwLock::new(access_token));
    let token = token_arc.read().await.clone();
    let res = client
        .get(format!(
            "https://api.spotify.com/v1/search?q={}&type=artist&limit=1",
            urlencoding::encode(artist)
        ))
        .bearer_auth(&token)
        .send()
        .await?
        .text()
        .await?;

    let json: Value = serde_json::from_str(&res)?;
    let mut artist_id = String::new();
    if let Some(items) = json["artists"]["items"].as_array() {
        if let Some(first_item) = items.first() {
            if let Some(uri) = first_item["uri"].as_str() {
                artist_id = uri.split(':').nth(2).unwrap().to_string();
            }
        }
    }

    Ok(artist_id)
}

async fn process_artists(
    page: u64,
    pool: Pool<SqliteConnectionManager>,
    existing_artists: Arc<RwLock<HashMap<String, u64>>>,
) -> Result<(), Box<dyn Error>> {
    let response = fetch_artists(page).await?;
    let artists = &response.artists["artist"];
    let existing_artists = existing_artists.read().await;

    if let Some(artists_array) = artists.as_array() {
        for artist in artists_array {
            if let (Some(name), Some(playcount_str)) = (
                artist.get("name").and_then(|v| v.as_str()),
                artist.get("playcount").and_then(|v| v.as_str()),
            ) {
                if let Ok(playcount) = playcount_str.parse::<u64>() {
                    if let Some(&existing_playcount) = existing_artists.get(name) {
                        if playcount != existing_playcount {
                            update_artist_playcount(&pool, name, playcount)?;
                            println!(
                                "Playcount updated for artist {}: {} -> {}",
                                name, existing_playcount, playcount
                            );
                        }
                    } else if let Ok(seed) = get_seed(name).await {
                        add_artist(&pool, name, playcount, &seed)?;
                        println!(
                            "Added artist: {} with playcount: {} and seed: {}",
                            name, playcount, &seed
                        );
                    }
                } else {
                    println!("Failed to parse playcount for artist: {}", name);
                }
            }
        }
    }

    Ok(())
}


