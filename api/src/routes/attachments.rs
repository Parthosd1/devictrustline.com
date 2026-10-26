//! Photos, receipts and documents attached to items. Files are kept in a private directory
//! (ATTACHMENTS_DIR/<workspace>/<id>) and only ever served through an access-checked route.
//! The file type is decided from the file's own bytes, not from what the browser claims.

use axum::body::Body as AxumBody;
use axum::extract::{Multipart, Path, State};
use axum::http::header;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::User;
use crate::db::Q;
use crate::error::{AppError, Result};
use crate::validate::path_uuid;
use crate::AppState;

pub const KINDS: &[&str] = &["photo", "receipt", "warranty", "manual", "other"];

/// Recognizes the formats people attach: photos and PDFs.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let starts = |sig: &[u8]| bytes.starts_with(sig);
    if starts(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if starts(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.len() > 12
        && &bytes[4..8] == b"ftyp"
        && [&b"heic"[..], b"heix", b"mif1", b"msf1"].contains(&&bytes[8..12])
    {
        Some("image/heic")
    } else if starts(b"%PDF-") {
        Some("application/pdf")
    } else {
        None
    }
}

fn file_path(state: &AppState, org: Uuid, id: Uuid) -> std::path::PathBuf {
    std::path::Path::new(&state.config.attachments_dir).join(org.to_string()).join(id.to_string())
}

/// A filename safe to show and to put in a download header.
fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control() && *c != '"').take(160).collect();
    if cleaned.trim().is_empty() {
        "file".into()
    } else {
        cleaned.trim().to_string()
    }
}

const SELECT: &str = r#"SELECT f.id, f.asset_id AS "assetId", f.kind, f.filename, f.content_type AS "contentType",
       f.size_bytes AS "size", f.created_at AS "createdAt", u.name AS "uploadedBy"
  FROM attachments f LEFT JOIN users u ON u.id = f.uploaded_by"#;

pub async fn list(State(state): State<AppState>, user: User, Path(asset): Path<String>) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let asset = path_uuid(&asset)?;
    let mut db = state.pool.acquire().await?;
    crate::routes::assets::find_asset(&mut db, user.org_id, asset).await?;
    let items = Q::new(format!("{SELECT} WHERE f.org_id = $1 AND f.asset_id = $2 ORDER BY f.created_at"))
        .bind(user.org_id)
        .bind(asset)
        .all(&mut db)
        .await?;
    Ok(Json(json!({ "items": items })))
}

/// Upload: multipart form with a `file` part and an optional `kind` part.
pub async fn upload(
    State(state): State<AppState>,
    user: User,
    Path(asset): Path<String>,
    mut form: Multipart,
) -> Result<(StatusCode, Json<Value>)> {
    user.require("assets:write")?;
    let asset = path_uuid(&asset)?;
    let too_big =
        || AppError::new(413, format!("Files can be up to {} MB", state.config.max_upload_bytes / 1024 / 1024));
    let mut kind = "other".to_string();
    let mut file: Option<(String, Vec<u8>)> = None;
    while let Some(mut field) = form.next_field().await.map_err(|_| AppError::new(400, "Upload could not be read"))? {
        match field.name() {
            Some("kind") => {
                let k = field.text().await.map_err(|_| AppError::new(400, "Upload could not be read"))?;
                if !KINDS.contains(&k.as_str()) {
                    return Err(AppError::new(400, format!("kind must be one of: {}", KINDS.join(", "))));
                }
                kind = k;
            }
            Some("file") => {
                let name = clean_name(field.file_name().unwrap_or("file"));
                let mut bytes = vec![];
                while let Some(chunk) = field.chunk().await.map_err(|_| too_big())? {
                    bytes.extend_from_slice(&chunk);
                    if bytes.len() > state.config.max_upload_bytes {
                        return Err(too_big());
                    }
                }
                file = Some((name, bytes));
            }
            _ => {}
        }
    }
    let (filename, bytes) =
        file.filter(|(_, b)| !b.is_empty()).ok_or_else(|| AppError::new(400, "Choose a file to upload"))?;
    let content_type =
        sniff(&bytes).ok_or_else(|| AppError::new(415, "Upload a photo (JPEG, PNG, GIF, WebP, HEIC) or a PDF"))?;

    let mut tx = state.pool.begin().await?;
    crate::routes::assets::find_asset(&mut tx, user.org_id, asset).await?;
    crate::routes::plans::check_storage(&mut tx, user.org_id, bytes.len() as i64).await?;
    let id: Uuid = Q::new(
        "INSERT INTO attachments (org_id, asset_id, kind, filename, content_type, size_bytes, sha256, uploaded_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(user.org_id)
    .bind(asset)
    .bind(&kind)
    .bind(&filename)
    .bind(content_type)
    .bind(bytes.len() as i64)
    .bind(Sha256::digest(&bytes).to_vec())
    .bind(user.id)
    .scalar(&mut tx)
    .await?;
    // Written before the row commits; a crash in between leaves only an orphan file, never a
    // row without its file.
    let path = file_path(&state, user.org_id, id);
    let dir = path.parent().unwrap().to_path_buf();
    let tmp = dir.join(format!(".{id}.part"));
    let write = async {
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await
    };
    write.await.map_err(|e| AppError::Internal(format!("could not store attachment: {e}")))?;
    crate::routes::assets::record_event(
        &mut tx,
        user.org_id,
        asset,
        user.id,
        "attachment_added",
        json!({ "attachment": { "from": null, "to": { "id": id, "kind": kind, "filename": filename } } }),
    )
    .await?;
    let row = Q::new(format!("{SELECT} WHERE f.id = $1")).bind(id).one(&mut tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn download(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Response> {
    user.require("assets:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let row = Q::new("SELECT filename, content_type FROM attachments WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .opt(&mut db)
        .await?
        .ok_or_else(|| AppError::not_found("Attachment"))?;
    let bytes = tokio::fs::read(file_path(&state, user.org_id, id)).await.map_err(|_| {
        AppError::new(410, "This file is missing from storage. Restore it from backup or upload it again.")
    })?;
    let content_type = row["content_type"].as_str().unwrap_or("application/octet-stream").to_string();
    let name: String = row["filename"].as_str().unwrap_or("file").chars().filter(char::is_ascii).collect();
    let disposition = format!("inline; filename=\"{name}\"");
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, "private, max-age=300".to_string()),
        ],
        AxumBody::from(bytes),
    )
        .into_response())
}

pub async fn delete(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    let row = Q::new("DELETE FROM attachments WHERE org_id = $1 AND id = $2 RETURNING asset_id, kind, filename")
        .bind(user.org_id)
        .bind(id)
        .opt(&mut tx)
        .await?
        .ok_or_else(|| AppError::not_found("Attachment"))?;
    let asset = crate::db::uuid_of(&row["asset_id"]).unwrap();
    let removed = json!({ "id": id, "kind": row["kind"], "filename": row["filename"] });
    crate::routes::assets::record_event(
        &mut tx,
        user.org_id,
        asset,
        user.id,
        "attachment_removed",
        json!({ "attachment": { "from": removed, "to": null } }),
    )
    .await?;
    tx.commit().await?;
    let _ = tokio::fs::remove_file(file_path(&state, user.org_id, id)).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_types_come_from_content() {
        assert_eq!(sniff(b"\xFF\xD8\xFF\xE0rest"), Some("image/jpeg"));
        assert_eq!(sniff(b"%PDF-1.7"), Some("application/pdf"));
        assert_eq!(sniff(b"<html><script>"), None);
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("C:\\Users\\a\\receipt \"1\".pdf"), "receipt 1.pdf");
    }
}
