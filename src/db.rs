use r2d2::{self, Pool};
use r2d2_sqlite::SqliteConnectionManager;
use r2d2_sqlite::rusqlite::{params, Result};

pub type DbPool = Pool<SqliteConnectionManager>;
pub fn establish_db_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    let manager = SqliteConnectionManager::file("data/artists.db");
    let pool = Pool::new(manager)?;
    Ok(pool)
}

pub fn create_table(pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS artists (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            playcount INTEGER NOT NULL,
            seed TEXT
        )",
        [],
    )?;
    Ok(())
}

pub fn add_artist(
    pool: &DbPool,
    artist_name: &str,
    playcount: u64,
    seed: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT OR IGNORE INTO artists (name, playcount, seed) VALUES (?1, ?2, ?3)",
        params![artist_name, playcount, seed],
    )?;

    Ok(())
}

pub fn update_artist_playcount(
    pool: &DbPool,
    artist_name: &str,
    playcount: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE artists SET playcount = ?1 WHERE name = ?2",
        params![playcount, artist_name],
    )?;
    Ok(())
}

