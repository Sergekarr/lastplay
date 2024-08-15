use r2d2::{self, Pool};
use r2d2_sqlite::rusqlite::Result;
use r2d2_sqlite::SqliteConnectionManager;

pub type DbPool = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;
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
