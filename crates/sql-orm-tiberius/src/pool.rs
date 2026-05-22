use crate::{MssqlConnection, MssqlConnectionConfig, MssqlPoolOptions, TokioConnectionStream};
use async_trait::async_trait;
use bb8::{ManageConnection, Pool, PooledConnection};
use core::ops::{Deref, DerefMut};
use sql_orm_core::OrmError;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct MssqlPool {
    inner: Pool<MssqlConnectionManager>,
    config: MssqlConnectionConfig,
    options: MssqlPoolOptions,
}

impl MssqlPool {
    pub fn builder() -> MssqlPoolBuilder {
        MssqlPoolBuilder::default()
    }

    pub async fn acquire(&self) -> Result<MssqlPooledConnection<'_>, OrmError> {
        let connection =
            self.inner.get().await.map_err(|_| {
                OrmError::connection("failed to acquire SQL Server pooled connection")
            })?;

        Ok(MssqlPooledConnection { inner: connection })
    }

    pub async fn acquire_owned(&self) -> Result<MssqlPooledConnection<'static>, OrmError> {
        let connection =
            self.inner.get_owned().await.map_err(|_| {
                OrmError::connection("failed to acquire SQL Server pooled connection")
            })?;

        Ok(MssqlPooledConnection { inner: connection })
    }

    pub fn max_size(&self) -> u32 {
        self.options.max_size
    }

    pub fn options(&self) -> &MssqlPoolOptions {
        &self.options
    }

    pub fn config(&self) -> &MssqlConnectionConfig {
        &self.config
    }
}

pub struct MssqlPooledConnection<'a> {
    inner: PooledConnection<'a, MssqlConnectionManager>,
}

impl Deref for MssqlPooledConnection<'_> {
    type Target = MssqlConnection<TokioConnectionStream>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for MssqlPooledConnection<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[derive(Debug, Clone)]
pub struct MssqlPoolBuilder {
    options: MssqlPoolOptions,
}

impl Default for MssqlPoolBuilder {
    fn default() -> Self {
        Self {
            options: MssqlPoolOptions::bb8(10),
        }
    }
}

impl MssqlPoolBuilder {
    pub fn max_size(mut self, max_size: u32) -> Self {
        self.options.max_size = max_size;
        self.options.enabled = true;
        self
    }

    pub fn min_idle(mut self, min_idle: u32) -> Self {
        self.options.min_idle = Some(min_idle);
        self.options.enabled = true;
        self
    }

    pub fn acquire_timeout(mut self, timeout: Duration) -> Self {
        self.options.acquire_timeout = Some(timeout);
        self.options.enabled = true;
        self
    }

    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.options.idle_timeout = Some(timeout);
        self.options.enabled = true;
        self
    }

    pub fn max_lifetime(mut self, timeout: Duration) -> Self {
        self.options.max_lifetime = Some(timeout);
        self.options.enabled = true;
        self
    }

    pub fn with_pool_options(mut self, options: MssqlPoolOptions) -> Self {
        self.options = options;
        self.options.enabled = true;
        self
    }

    pub fn options(&self) -> MssqlPoolOptions {
        self.options
    }

    pub async fn connect(self, connection_string: &str) -> Result<MssqlPool, OrmError> {
        let config = MssqlConnectionConfig::from_connection_string(connection_string)?;
        self.connect_with_config(config).await
    }

    pub async fn connect_with_config(
        self,
        config: MssqlConnectionConfig,
    ) -> Result<MssqlPool, OrmError> {
        build_pool(config, self.options).await
    }
}

#[derive(Debug, Clone)]
pub struct MssqlConnectionManager {
    config: MssqlConnectionConfig,
}

impl MssqlConnectionManager {
    fn new(config: MssqlConnectionConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ManageConnection for MssqlConnectionManager {
    type Connection = MssqlConnection<TokioConnectionStream>;
    type Error = OrmError;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        MssqlConnection::connect_with_config(self.config.clone()).await
    }

    async fn is_valid(&self, connection: &mut Self::Connection) -> Result<(), Self::Error> {
        connection.health_check().await
    }

    fn has_broken(&self, _connection: &mut Self::Connection) -> bool {
        false
    }
}

async fn build_pool(
    config: MssqlConnectionConfig,
    options: MssqlPoolOptions,
) -> Result<MssqlPool, OrmError> {
    let operational_options = config.options().clone().with_pool(options);
    let config = config.with_options(operational_options);
    let manager = MssqlConnectionManager::new(config.clone());
    let mut builder = Pool::builder().max_size(options.max_size);

    if let Some(min_idle) = options.min_idle {
        builder = builder.min_idle(Some(min_idle));
    }

    if let Some(timeout) = options.acquire_timeout {
        builder = builder.connection_timeout(timeout);
    }

    if let Some(timeout) = options.idle_timeout {
        builder = builder.idle_timeout(Some(timeout));
    }

    if let Some(timeout) = options.max_lifetime {
        builder = builder.max_lifetime(Some(timeout));
    }

    let inner = builder
        .build(manager)
        .await
        .map_err(|_| OrmError::connection("failed to create SQL Server connection pool"))?;

    Ok(MssqlPool {
        inner,
        config,
        options,
    })
}

#[cfg(test)]
mod tests {
    use super::MssqlPool;
    use crate::{MssqlPoolBackend, MssqlPoolOptions};
    use std::time::Duration;

    #[test]
    fn builder_starts_with_bb8_defaults() {
        let builder = MssqlPool::builder();

        assert_eq!(builder.options().backend, MssqlPoolBackend::Bb8);
        assert_eq!(builder.options().max_size, 10);
        assert!(builder.options().enabled);
    }

    #[test]
    fn builder_preserves_explicit_pool_options() {
        let builder = MssqlPool::builder()
            .max_size(16)
            .min_idle(4)
            .acquire_timeout(Duration::from_secs(2))
            .idle_timeout(Duration::from_secs(30))
            .max_lifetime(Duration::from_secs(300));

        assert_eq!(builder.options().backend, MssqlPoolBackend::Bb8);
        assert_eq!(builder.options().max_size, 16);
        assert_eq!(builder.options().min_idle, Some(4));
        assert_eq!(
            builder.options().acquire_timeout,
            Some(Duration::from_secs(2))
        );
        assert_eq!(
            builder.options().idle_timeout,
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            builder.options().max_lifetime,
            Some(Duration::from_secs(300))
        );
    }

    #[test]
    fn builder_can_replace_options_wholesale() {
        let options = MssqlPoolOptions::bb8(12).with_min_idle(3);
        let builder = MssqlPool::builder().with_pool_options(options);

        assert_eq!(builder.options(), options);
    }
}
