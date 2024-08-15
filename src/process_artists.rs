use crate::db::{create_table, establish_db_pool, DbPool};
use crate::spotify_auth::{get_key, get_redis_pool};
use futures::future::join_all;
use reqwest::Client;
use rusqlite::params;
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

struct ArtistProcessor {
    pool: Arc<DbPool>,
    existing_artists: Arc<RwLock<HashMap<String, u64>>>,
}

impl ArtistProcessor {
    async fn new() -> Result<Self, Box<dyn Error>> {
        let pool = establish_db_pool()?;
        create_table(&pool)?;
        let pool = Arc::new(pool);
        let existing_artists = Self::get_existing_artists(&pool).await?;
        Ok(Self {
            pool,
            existing_artists: Arc::new(RwLock::new(existing_artists)),
        })
    }

    async fn get_existing_artists(pool: &DbPool) -> Result<HashMap<String, u64>, Box<dyn Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare("SELECT name, playcount FROM artists")?;
        let artist_iter = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut existing_artists = HashMap::new();
        for artist in artist_iter {
            let (name, playcount): (String, u64) = artist?;
            existing_artists.insert(name, playcount);
        }
        Ok(existing_artists)
    }

    async fn fetch_artists_page(page: u64) -> Result<Vec<Value>, Box<dyn Error>> {
        let username = env::var("LASTFM_USERNAME").expect("LASTFM_USERNAME must be set");
        let api_key = env::var("LASTFM_API_KEY").expect("LASTFM_API_KEY must be set");
        let url = format!(
            "http://ws.audioscrobbler.com/2.0/?method=library.getartists&api_key={}&limit=2000&page={}&user={}&format=json",
            api_key, page, username,
        );
        let response_text = reqwest::get(&url).await?.text().await?;
        let response: Response = serde_json::from_str(&response_text)?;
        Ok(response.artists["artist"]
            .as_array()
            .unwrap_or(&Vec::new())
            .to_vec())
    }

    async fn get_total_pages() -> Result<u64, Box<dyn Error>> {
        let username = env::var("LASTFM_USERNAME").expect("LASTFM_USERNAME must be set");
        let api_key = env::var("LASTFM_API_KEY").expect("LASTFM_API_KEY must be set");
        let url = format!(
            "http://ws.audioscrobbler.com/2.0/?method=library.getartists&api_key={}&limit=2000&user={}&format=json",
            api_key, username
        );
        let response_text = reqwest::get(&url).await?.text().await?;
        let response: Response = serde_json::from_str(&response_text)?;
        Ok(response.artists["@attr"]["totalPages"]
            .as_str()
            .unwrap()
            .parse()?)
    }

    async fn get_seed(artist: &str) -> Result<String, Box<dyn Error>> {
        let client = Client::new();
        let pool = get_redis_pool().await;
        let access_token = get_key(pool, "access_token").await?;

        let res = client
            .get(format!(
                "https://api.spotify.com/v1/search?q={}&type=artist&limit=1",
                urlencoding::encode(artist)
            ))
            .bearer_auth(&access_token)
            .send()
            .await?
            .text()
            .await?;

        let json: Value = serde_json::from_str(&res)?;
        let artist_id = json["artists"]["items"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item["uri"].as_str())
            .and_then(|uri| uri.split(':').nth(2))
            .unwrap_or("")
            .to_string();

        Ok(artist_id)
    }

    async fn dbupdate(
        &self,
        artist_name: &str,
        playcount: u64,
        seed: &str,
    ) -> Result<(), Box<dyn Error>> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT OR REPLACE INTO artists (name, playcount, seed) VALUES (?1, ?2, ?3)",
            params![artist_name, playcount, seed],
        )?;
        Ok(())
    }

    async fn process_artist(&self, artist: &Value) -> Result<(), Box<dyn Error>> {
        if let (Some(name), Some(playcount_str)) =
            (artist["name"].as_str(), artist["playcount"].as_str())
        {
            if let Ok(playcount) = playcount_str.parse::<u64>() {
                let mut existing_artists = self.existing_artists.write().await;
                let existing_playcount = existing_artists.get(name).cloned();

                if existing_playcount.map_or(true, |count| playcount != count) {
                    let seed = if let Some(existing_seed) = existing_artists.get(name) {
                        existing_seed.to_string()
                    } else {
                        Self::get_seed(name).await?
                    };

                    self.dbupdate(name, playcount, &seed).await?;
                    existing_artists.insert(name.to_string(), playcount);
                    println!(
                        "{} artist: {} with playcount: {} and seed: {}",
                        if existing_playcount.is_some() {
                            "Updated"
                        } else {
                            "Added"
                        },
                        name,
                        playcount,
                        seed
                    );
                }
            } else {
                println!("Failed to parse playcount for artist: {}", name);
            }
        }
        Ok(())
    }

    async fn process_page(&self, page: u64) -> Result<(), Box<dyn Error>> {
        let artists = Self::fetch_artists_page(page).await?;
        for artist in artists {
            if let Err(e) = self.process_artist(&artist).await {
                eprintln!("Error processing artist: {:?}", e);
            }
        }
        Ok(())
    }

    async fn process_artists(&self) -> Result<(), Box<dyn Error>> {
        let total_pages = Self::get_total_pages().await?;
        let mut handles = Vec::new();

        for page in 1..=total_pages {
            let self_clone = self.clone();
            handles.push(tokio::spawn(async move {
                if let Err(e) = self_clone.process_page(page).await {
                    eprintln!("Error processing page {}: {:?}", page, e);
                }
            }));
        }

        join_all(handles).await;
        Ok(())
    }
}

impl Clone for ArtistProcessor {
    fn clone(&self) -> Self {
        Self {
            pool: Arc::clone(&self.pool),
            existing_artists: Arc::clone(&self.existing_artists),
        }
    }
}

pub async fn process() -> Result<(), Box<dyn Error>> {
    let processor = ArtistProcessor::new().await?;
    processor.process_artists().await
}
