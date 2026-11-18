//! Where finished recordings are kept besides local disk: a private object store. The trait is
//! synchronous because jobs and purges run on blocking threads; the server implements it over S3.
use anyhow::Result;
use std::path::Path;

pub trait Blobs: Send + Sync {
    /// Uploads the file; replaces an existing object with the same key.
    fn put_file(&self, key: &str, file: &Path) -> Result<()>;
    /// Deletes the object. Deleting one that is already gone succeeds.
    fn delete(&self, key: &str) -> Result<()>;
}

/// Object key of a meeting's recording: tenant first, so one bucket serves every tenant.
pub fn audio_key(tenant_id: &str, audio_path: &str) -> String {
    format!("{tenant_id}/{audio_path}")
}
