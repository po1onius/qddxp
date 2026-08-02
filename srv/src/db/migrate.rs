use diesel::{Connection, PgConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

type MigrationError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub fn run_pending(database_url: &str) -> Result<(), MigrationError> {
    let mut conn = PgConnection::establish(database_url)?;
    let ran_migrations = conn.run_pending_migrations(MIGRATIONS)?;

    if ran_migrations.is_empty() {
        tracing::info!("database migrations are up to date");
    } else {
        tracing::info!(
            count = ran_migrations.len(),
            "ran pending database migrations"
        );
    }

    Ok(())
}
