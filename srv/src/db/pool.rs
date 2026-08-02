use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, bb8::Pool},
};

pub type DbPool = Pool<AsyncPgConnection>;

pub async fn create_pool(
    database_url: &str,
) -> Result<DbPool, diesel_async::pooled_connection::bb8::RunError> {
    let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(database_url);
    Pool::builder().build(manager).await.map_err(Into::into)
}
