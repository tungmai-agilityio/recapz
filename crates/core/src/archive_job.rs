//! `archive` job: moves a finished recording to object storage, then removes the local file.
use crate::{
    blobs::{audio_key, Blobs},
    db::tenant_dir,
    jobs::Job,
};
use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use std::{fs, io, path::Path};

pub const KIND: &str = "archive";

/// Safe to retry: the upload overwrites, the flag flips only after it succeeds, and a recording
/// already marked remote is left alone. Only the audio file goes; `speakers.jsonl` stays for labelling.
pub fn handle(
    c: &Connection,
    job: &Job,
    data_dir: &Path,
    tenant_id: &str,
    blobs: &dyn Blobs,
) -> Result<()> {
    let mid = job
        .meeting_id
        .as_deref()
        .ok_or_else(|| anyhow!("job has no meeting"))?;
    let (rel, remote): (Option<String>, bool) = c
        .query_row(
            "SELECT audio_path, audio_remote FROM meetings WHERE id=?1",
            [mid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .context("meeting is gone")?;
    let Some(rel) = rel.filter(|_| !remote) else {
        return Ok(());
    };
    let file = tenant_dir(data_dir, tenant_id).join(&rel);
    // The key is stored in the error path only as the meeting id, never the file name or contents.
    blobs
        .put_file(&audio_key(tenant_id, &rel), &file)
        .with_context(|| format!("upload recording of {mid}"))?;
    if c.execute("UPDATE meetings SET audio_remote=1 WHERE id=?1", [mid])? == 0 {
        // The meeting was deleted while the upload ran, so the purge saw no remote copy and skipped
        // the object. Remove it here rather than leave audio nobody can reach or delete.
        return blobs.delete(&audio_key(tenant_id, &rel));
    }
    match fs::remove_file(&file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{db::open_tenant, jobs};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Fake {
        pub objects: Mutex<Vec<(String, Vec<u8>)>>,
        pub fail: Mutex<bool>,
    }
    impl Blobs for Fake {
        fn put_file(&self, key: &str, file: &Path) -> Result<()> {
            if *self.fail.lock().unwrap() {
                anyhow::bail!("store down");
            }
            let mut o = self.objects.lock().unwrap();
            o.retain(|(k, _)| k != key);
            o.push((key.into(), fs::read(file)?));
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.objects.lock().unwrap().retain(|(k, _)| k != key);
            Ok(())
        }
    }

    fn setup() -> (tempfile::TempDir, Connection, Job) {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        let a = tenant_dir(dir.path(), "t1").join("audio/m1");
        fs::create_dir_all(&a).unwrap();
        fs::write(a.join("recording.wav"), b"RIFFdata").unwrap();
        fs::write(a.join("speakers.jsonl"), b"{}").unwrap();
        c.execute(
            "INSERT INTO meetings (id, title, platform, status, audio_path) VALUES ('m1','t','meet','ready','audio/m1/recording.wav')",
            [],
        )
        .unwrap();
        jobs::enqueue(&c, KIND, Some("m1"), None).unwrap();
        let job = jobs::claim(&c, &[KIND]).unwrap().unwrap();
        (dir, c, job)
    }

    fn remote(c: &Connection) -> bool {
        c.query_row("SELECT audio_remote FROM meetings WHERE id='m1'", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn uploads_then_flags_and_removes_only_the_audio_file() {
        let (dir, c, job) = setup();
        let b = Fake::default();
        handle(&c, &job, dir.path(), "t1", &b).unwrap();
        assert_eq!(
            *b.objects.lock().unwrap(),
            vec![(
                "t1/audio/m1/recording.wav".to_string(),
                b"RIFFdata".to_vec()
            )]
        );
        assert!(remote(&c));
        let a = tenant_dir(dir.path(), "t1").join("audio/m1");
        assert!(!a.join("recording.wav").exists());
        assert!(a.join("speakers.jsonl").exists());
        // a rerun is a no-op, not an error and not a second object
        handle(&c, &job, dir.path(), "t1", &b).unwrap();
        assert_eq!(b.objects.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_meeting_deleted_during_the_upload_leaves_no_object_behind() {
        struct DeleteWhileUploading<'a>(&'a Path, &'a Fake);
        impl Blobs for DeleteWhileUploading<'_> {
            fn put_file(&self, key: &str, file: &Path) -> Result<()> {
                self.1.put_file(key, file)?;
                // the purge runs now: it sees audio_remote = 0, so it deletes no object
                let other = crate::db::open_tenant(self.0, "t1")?;
                other.execute("DELETE FROM meetings WHERE id='m1'", [])?;
                Ok(())
            }
            fn delete(&self, key: &str) -> Result<()> {
                self.1.delete(key)
            }
        }
        let (dir, c, job) = setup();
        let fake = Fake::default();
        handle(
            &c,
            &job,
            dir.path(),
            "t1",
            &DeleteWhileUploading(dir.path(), &fake),
        )
        .unwrap();
        assert!(fake.objects.lock().unwrap().is_empty(), "object removed");
    }

    #[test]
    fn a_failed_upload_keeps_the_local_file_and_the_flag_off() {
        let (dir, c, job) = setup();
        let b = Fake::default();
        *b.fail.lock().unwrap() = true;
        assert!(handle(&c, &job, dir.path(), "t1", &b).is_err());
        assert!(!remote(&c));
        assert!(tenant_dir(dir.path(), "t1")
            .join("audio/m1/recording.wav")
            .exists());
    }
}
