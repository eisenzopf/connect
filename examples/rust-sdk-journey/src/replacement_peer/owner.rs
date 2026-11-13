//! Local fixture ownership only: no service mutation or operation retry.
use super::*;
use tokio::io::AsyncReadExt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stop {
    command: String,
    operation_id: String,
}

fn validate(bytes: &[u8], original: &str) -> Result<()> {
    let value: Stop = serde_json::from_slice(bytes)?;
    if value.command != "stop" || value.operation_id != original {
        return Err("physical owner stop changed original operation".into());
    }
    Ok(())
}

pub(super) async fn stop(output: &std::path::Path, original: &str) -> Result<bool> {
    let path = output.with_extension("owner-stop.json");
    let info = match tokio::fs::symlink_metadata(&path).await {
        Ok(info) => info,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !info.is_file() || info.len() > 1024 {
        return Err("physical owner stop exceeds private file bounds".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if info.permissions().mode() & 0o077 != 0 {
            return Err("physical owner stop is not private".into());
        }
    }
    let mut bytes = Vec::new();
    tokio::fs::File::open(path)
        .await?
        .take(1025)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 1024 {
        return Err("physical owner stop exceeds read bound".into());
    }
    validate(&bytes, original)?;
    Ok(true)
}

pub(super) async fn ready(
    output: &std::path::Path,
    original: &str,
    connection: &str,
    streams: &[Value],
    video: bool,
) -> Result<()> {
    let path = output.with_extension("owner-ready.json");
    let temporary = output.with_extension("owner-ready.tmp");
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).await?;
    file.write_all(&serde_json::to_vec(&json!({
        "format":"connect/native-replacement-owner-ready/1", "operation_id":original,
        "connection_id":connection, "negotiated_streams":streams, "video_enabled":video,
        "decoded_media_verified":false, "operation_completed_inferred":false
    }))?)
    .await?;
    file.sync_all().await?;
    drop(file);
    // Publish only a complete immutable marker; readers cannot see partial JSON.
    tokio::fs::hard_link(&temporary, &path).await?;
    tokio::fs::remove_file(temporary).await?;
    tokio::fs::File::open(path.parent().ok_or("private owner directory missing")?)
        .await?
        .sync_all()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn private_owner_markers_publish_complete_bytes_and_refuse_foreign_or_oversized_stop() {
        let directory = std::env::current_dir()
            .unwrap()
            .join(".local")
            .join(format!(
                "replacement-owner-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let output = directory.join("observation.json");
        assert!(!stop(&output, "original").await.unwrap());
        ready(
            &output,
            "original",
            "candidate",
            &[json!({"strm_id":"audio"})],
            false,
        )
        .await
        .unwrap();
        let saved: Value = serde_json::from_slice(
            &tokio::fs::read(output.with_extension("owner-ready.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["operation_id"], "original");
        assert_eq!(saved["connection_id"], "candidate");
        assert_eq!(saved["negotiated_streams"], json!([{"strm_id":"audio"}]));
        assert!(ready(&output, "another", "candidate", &[], false)
            .await
            .is_err());
        let path = output.with_extension("owner-stop.json");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(&path).unwrap());
        tokio::fs::write(&path, br#"{"command":"stop","operation_id":"original"}"#)
            .await
            .unwrap();
        assert!(stop(&output, "original").await.unwrap());
        assert!(stop(&output, "another").await.is_err());
        tokio::fs::write(&path, vec![b' '; 1025]).await.unwrap();
        assert!(stop(&output, "original").await.is_err());
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[test]
    fn owner_stop_is_only_for_the_exact_original_and_has_no_mutation_fields() {
        assert!(validate(
            br#"{"command":"stop","operation_id":"original"}"#,
            "original"
        )
        .is_ok());
        for bytes in [
            br#"{"command":"stop","operation_id":"another"}"#.as_slice(),
            br#"{"command":"retry","operation_id":"original"}"#,
            br#"{"command":"stop","operation_id":"original","new_operation_id":"new"}"#,
            br#"{"command":"stop","operation_id":"original""#,
        ] {
            assert!(validate(bytes, "original").is_err());
        }
    }
}
