//! Redis-backed adapter for the checkpointer RedisStore seam.

use std::sync::Mutex;

use ah_contracts::checkpointer::{
    CheckpointerError, PipelineOp, PipelineResult, RedisPipeline, RedisStore, RedisValue,
};
use ah_contracts::seam::Seam;
use redis::{Commands, Connection};

pub struct RedisCheckpointerStore {
    connection: Mutex<Connection>,
}

impl RedisCheckpointerStore {
    pub fn open(url: &str) -> Result<Self, CheckpointerError> {
        let client = redis::Client::open(url)
            .map_err(|error| CheckpointerError(format!("open checkpointer redis: {error}")))?;
        let connection = client
            .get_connection()
            .map_err(|error| CheckpointerError(format!("connect checkpointer redis: {error}")))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn bytes(value: RedisValue) -> Vec<u8> {
        match value {
            RedisValue::Str(value) => value.into_bytes(),
            RedisValue::Bytes(value) => value,
        }
    }

    fn value(raw: Option<Vec<u8>>) -> Option<RedisValue> {
        raw.map(RedisValue::Bytes)
    }

    fn keys(&self, prefix: &str) -> Result<Vec<String>, CheckpointerError> {
        let pattern = format!("{prefix}*");
        let mut connection = self.connection.lock().unwrap();
        redis::cmd("KEYS")
            .arg(pattern.clone())
            .query(&mut *connection)
            .map_err(|error| CheckpointerError(format!("redis KEYS {pattern}: {error}")))
    }
}

impl Seam for RedisCheckpointerStore {}

impl RedisStore for RedisCheckpointerStore {
    fn set(&self, key: &str, value: RedisValue) -> Result<(), CheckpointerError> {
        let mut connection = self.connection.lock().unwrap();
        connection
            .set::<_, _, ()>(key, Self::bytes(value))
            .map_err(|error| CheckpointerError(format!("redis SET {key}: {error}")))
    }

    fn exclusive_set(
        &self,
        key: &str,
        value: RedisValue,
        expiry_seconds: Option<i64>,
    ) -> Result<bool, CheckpointerError> {
        let mut connection = self.connection.lock().unwrap();
        let mut command = redis::cmd("SET");
        command.arg(key).arg(Self::bytes(value)).arg("NX");
        if let Some(expiry) = expiry_seconds.filter(|expiry| *expiry > 0) {
            command.arg("EX").arg(expiry);
        }
        let result: Option<String> = command
            .query(&mut *connection)
            .map_err(|error| CheckpointerError(format!("redis SET NX {key}: {error}")))?;
        Ok(result.is_some())
    }

    fn get(&self, key: &str) -> Result<Option<RedisValue>, CheckpointerError> {
        let mut connection = self.connection.lock().unwrap();
        let value: Option<Vec<u8>> = connection
            .get(key)
            .map_err(|error| CheckpointerError(format!("redis GET {key}: {error}")))?;
        Ok(Self::value(value))
    }

    fn exists(&self, key: &str) -> Result<bool, CheckpointerError> {
        let mut connection = self.connection.lock().unwrap();
        connection
            .exists(key)
            .map_err(|error| CheckpointerError(format!("redis EXISTS {key}: {error}")))
    }

    fn delete(&self, key: &str) -> Result<i64, CheckpointerError> {
        let mut connection = self.connection.lock().unwrap();
        connection
            .del(key)
            .map_err(|error| CheckpointerError(format!("redis DEL {key}: {error}")))
    }

    fn get_by_prefix(&self, prefix: &str) -> Result<Vec<(String, RedisValue)>, CheckpointerError> {
        let keys = self.keys(prefix)?;
        keys.into_iter()
            .map(|key| self.get(&key).map(|value| value.map(|value| (key, value))))
            .filter_map(|result| match result {
                Ok(Some(entry)) => Some(Ok(entry)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    fn delete_by_prefix(&self, prefix: &str, batch_size: i64) -> Result<i64, CheckpointerError> {
        let keys = self.keys(prefix)?;
        if keys.is_empty() {
            return Ok(0);
        }
        if batch_size <= 0 {
            return self.batch_delete(&keys, 0);
        }
        let mut deleted = 0;
        for batch in keys.chunks(batch_size as usize) {
            deleted += self.batch_delete(batch, batch.len() as i64)?;
        }
        Ok(deleted)
    }

    fn mget(&self, keys: &[String]) -> Result<Vec<Option<RedisValue>>, CheckpointerError> {
        keys.iter().map(|key| self.get(key)).collect()
    }

    fn batch_delete(&self, keys: &[String], batch_size: i64) -> Result<i64, CheckpointerError> {
        if keys.is_empty() {
            return Ok(0);
        }
        if batch_size > 0 && keys.len() > batch_size as usize {
            return keys
                .chunks(batch_size as usize)
                .try_fold(0, |total, batch| {
                    Ok(total + self.batch_delete(batch, batch.len() as i64)?)
                });
        }
        let mut connection = self.connection.lock().unwrap();
        connection
            .del(keys)
            .map_err(|error| CheckpointerError(format!("redis DEL batch: {error}")))
    }

    fn refresh_ttl(&self, keys: &[String], ttl_seconds: i64) {
        if keys.is_empty() || ttl_seconds <= 0 {
            return;
        }
        if let Ok(mut connection) = self.connection.lock() {
            for key in keys {
                let _: Result<(), _> = connection.expire(key, ttl_seconds);
            }
        }
    }

    fn pipeline(&self, pipeline: &RedisPipeline) -> Result<PipelineResult, CheckpointerError> {
        pipeline
            .ops()
            .iter()
            .map(|operation| match operation {
                PipelineOp::Set {
                    key,
                    value,
                    ttl_seconds,
                } => {
                    self.set(key, value.clone())?;
                    if let Some(ttl) = ttl_seconds.filter(|ttl| *ttl > 0) {
                        let mut connection = self.connection.lock().unwrap();
                        let _: () = redis::cmd("EXPIRE")
                            .arg(key)
                            .arg(ttl)
                            .query(&mut *connection)
                            .map_err(|error| {
                                CheckpointerError(format!("redis EXPIRE {key}: {error}"))
                            })?;
                    }
                    Ok(None)
                }
                PipelineOp::Get { key } => self.get(key),
                PipelineOp::Exists { key } => self
                    .exists(key)
                    .map(|exists| exists.then_some(RedisValue::Str("1".to_string()))),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_conversion_preserves_string_and_bytes_payloads() {
        assert_eq!(
            RedisCheckpointerStore::bytes(RedisValue::Str("x".into())),
            b"x"
        );
        assert_eq!(
            RedisCheckpointerStore::bytes(RedisValue::Bytes(vec![0, 1])),
            vec![0, 1]
        );
        assert_eq!(
            RedisCheckpointerStore::value(Some(b"x".to_vec())),
            Some(RedisValue::Bytes(b"x".to_vec()))
        );
        assert_eq!(RedisCheckpointerStore::value(None), None);
    }
    #[test]
    fn redis_store_roundtrips_atomic_claim_prefix_and_pipeline() {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".into());
        let store = match RedisCheckpointerStore::open(&url) {
            Ok(store) => store,
            Err(error) => {
                println!("skipping: redis unavailable: {error}");
                return;
            }
        };
        let prefix = format!("ah-checkpointer-test:{}:", std::process::id());
        let key = format!("{prefix}state");
        let claim = format!("{prefix}claim");
        let pipeline_key = format!("{prefix}pipeline");
        store.delete_by_prefix(&prefix, 0).expect("cleanup before");

        store
            .set(&key, RedisValue::Str("payload".into()))
            .expect("set");
        assert_eq!(
            store.get(&key).expect("get"),
            Some(RedisValue::Bytes(b"payload".to_vec()))
        );
        assert!(store.exists(&key).expect("exists"));
        assert!(
            store
                .exclusive_set(&claim, RedisValue::Str("owner".into()), Some(30))
                .expect("claim")
        );
        assert!(
            !store
                .exclusive_set(&claim, RedisValue::Str("other".into()), Some(30))
                .expect("second claim")
        );

        let values = store
            .pipeline(
                &RedisPipeline::new()
                    .set(&pipeline_key, RedisValue::Str("p".into()), Some(30))
                    .get(&pipeline_key)
                    .exists(&pipeline_key),
            )
            .expect("pipeline");
        assert_eq!(values.len(), 3);
        assert_eq!(store.get_by_prefix(&prefix).expect("prefix").len(), 3);
        assert_eq!(
            store.delete_by_prefix(&prefix, 1).expect("delete prefix"),
            3
        );
        assert!(!store.exists(&key).expect("deleted"));
    }
}
