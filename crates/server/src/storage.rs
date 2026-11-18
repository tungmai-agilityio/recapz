//! Private S3-compatible bucket for finished recordings. Keys are `{tenant}/{audio_path}`; the
//! bucket is never public and every read goes through the server's meeting access check.
use crate::config::Storage;
use anyhow::{anyhow, Context, Result};
use recapz_core::blobs::Blobs;
use s3::{creds::Credentials, region::Region, Bucket};
use std::path::Path;
use tokio::runtime::Handle;

pub struct S3Store {
    bucket: Box<Bucket>,
    rt: Handle,
}

impl S3Store {
    /// `None` when no bucket is configured (recordings then stay on local disk).
    pub fn from_config(cfg: &Storage, rt: Handle) -> Result<Option<Self>> {
        if cfg.bucket.is_empty() {
            return Ok(None);
        }
        for (v, name) in [
            (&cfg.endpoint, "storage.endpoint"),
            (&cfg.region, "storage.region"),
            (&cfg.key_id, "RECAPZ_STORAGE_KEY_ID"),
            (&cfg.secret, "RECAPZ_STORAGE_SECRET"),
        ] {
            if v.trim().is_empty() {
                anyhow::bail!("{name} is required when storage.bucket is set");
            }
        }
        let creds = Credentials::new(Some(&cfg.key_id), Some(&cfg.secret), None, None, None)
            .map_err(|_| anyhow!("storage credentials"))?;
        let region = Region::Custom {
            region: cfg.region.clone(),
            endpoint: cfg.endpoint.clone(),
        };
        // Path style: the bucket name goes in the path, which every S3-compatible host accepts.
        let bucket = Bucket::new(&cfg.bucket, region, creds)
            .map_err(|_| anyhow!("storage bucket settings"))?
            .with_path_style();
        Ok(Some(Self { bucket, rt }))
    }

    /// Size in bytes, or `None` when the object does not exist.
    pub async fn size(&self, key: &str) -> Result<Option<u64>> {
        match self.bucket.head_object(key).await {
            Ok((h, 200)) => Ok(h.content_length.map(|n| n as u64)),
            Ok((_, 404)) | Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Ok(None),
            // 403, 5xx and the rest are the store failing, not the recording missing
            Ok((_, status)) => Err(anyhow!("object store: HTTP {status}")),
            Err(e) => Err(anyhow!("object store: {}", kind(&e))),
        }
    }

    /// The whole object as a byte stream.
    pub async fn stream(&self, key: &str) -> Result<s3::request::DataStream> {
        let r = self
            .bucket
            .get_object_stream(key)
            .await
            .map_err(|e| anyhow!("object store: {}", kind(&e)))?;
        Ok(r.bytes)
    }

    /// Bytes `start..=end` (inclusive), as a ranged GET.
    pub async fn range(&self, key: &str, start: u64, end: u64) -> Result<Vec<u8>> {
        let r = self
            .bucket
            .get_object_range(key, start, Some(end))
            .await
            .map_err(|e| anyhow!("object store: {}", kind(&e)))?;
        Ok(r.as_slice().to_vec())
    }
}

/// Error class only: the provider's message can echo the request, and logs hold no key names.
fn kind(e: &s3::error::S3Error) -> &'static str {
    match e {
        s3::error::S3Error::HttpFailWithBody(401 | 403, _) => "access denied",
        s3::error::S3Error::HttpFailWithBody(404, _) => "not found",
        s3::error::S3Error::HttpFailWithBody(_, _) => "request refused",
        _ => "request failed",
    }
}

impl Blobs for S3Store {
    fn put_file(&self, key: &str, file: &Path) -> Result<()> {
        self.rt.block_on(async {
            let mut f = tokio::fs::File::open(file)
                .await
                .context("open recording")?;
            self.bucket
                .put_object_stream_with_content_type(&mut f, key, "audio/wav")
                .await
                .map(|_| ())
                .map_err(|e| anyhow!("object store: {}", kind(&e)))
        })
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.rt.block_on(async {
            self.bucket
                .delete_object(key)
                .await
                .map(|_| ())
                .map_err(|e| anyhow!("object store: {}", kind(&e)))
        })
    }
}
