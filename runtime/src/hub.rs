//! Native Hugging Face dataset transfers, using the documented commit/LFS API.
//! Credentials stay in the user's normal HF token store and are never packaged.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use reqwest::blocking::{Body, Client, RequestBuilder, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};
const HOST: &str = "https://huggingface.co";
fn token() -> Option<String> {
    std::env::var("HF_TOKEN")
        .ok()
        .or_else(|| {
            let home = std::env::var_os("HF_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                        .join(".cache/huggingface")
                });
            fs::read_to_string(
                std::env::var_os("HF_TOKEN_PATH")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("token")),
            )
            .ok()
        })
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
}
fn auth(request: RequestBuilder, token: &Option<String>) -> RequestBuilder {
    if let Some(token) = token {
        request.bearer_auth(token)
    } else {
        request
    }
}
fn check(response: Response) -> Result<Response> {
    ensure!(
        response.status().is_success(),
        "Hub request failed: HTTP {}",
        response.status().as_u16()
    );
    Ok(response)
}
fn repo_id(repo: &str) -> Result<(&str, &str)> {
    let (owner, name) = repo.split_once('/').context("Use namespace/dataset")?;
    for part in [owner, name] {
        ensure!(
            !part.is_empty()
                && part.len() <= 96
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                && !part.contains("..")
                && !part.contains("--")
                && !part.starts_with(['-', '.'])
                && !part.ends_with(['-', '.']),
            "Invalid Hub repository name"
        );
    }
    ensure!(!name.ends_with(".git"), "Invalid Hub repository suffix");
    Ok((owner, name))
}
fn relative(path: &str) -> Result<&Path> {
    let p = Path::new(path);
    ensure!(
        !p.as_os_str().is_empty() && p.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe dataset path"
    );
    Ok(p)
}
fn ignored(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == ".cache"
            || s == ".staging"
            || s == ".git"
            || s == ".dorobot-writer.lock"
            || s.starts_with(".dorobot-commit")
            || s.ends_with(".tmp")
    })
}
fn files(root: &Path, path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let rel = entry.path().strip_prefix(root)?.to_owned();
        if ignored(&rel) {
            continue;
        }
        let ty = entry.file_type()?;
        ensure!(!ty.is_symlink(), "Dataset contains a symlink");
        if ty.is_dir() {
            files(root, &entry.path(), out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}
fn digest(path: &Path) -> Result<(u64, String, String)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut sample = [0; 512];
    let n = file.read(&mut sample)?;
    let sample = B64.encode(&sample[..n]);
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok((size, format!("{:x}", hash.finalize()), sample))
}
struct DownloadStage(PathBuf);
impl Drop for DownloadStage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn transfer(mode: &str, repo: &str, path: &Path) -> Result<()> {
    let (owner, name) = repo_id(repo)?;
    let token = token();
    let client = Client::builder()
        .user_agent("dorobot-studio-rust/0.1")
        .timeout(Duration::from_secs(900))
        .build()?;
    if mode == "pull" {
        ensure!(
            !path.exists() || fs::read_dir(path)?.next().is_none(),
            "Download destination must be empty"
        );
        let parent = path.parent().context("Download destination directory")?;
        fs::create_dir_all(parent)?;
        let stage = DownloadStage(parent.join(format!(
            ".dorobot-download-{}-{}",
            std::process::id(),
            crate::protocol::now_ns()
        )));
        fs::create_dir(&stage.0)?;
        let info: Value = check(
            auth(
                client.get(format!("{HOST}/api/datasets/{repo}/revision/main")),
                &token,
            )
            .send()?,
        )?
        .json()?;
        let revision = info["sha"].as_str().context("Dataset revision missing")?;
        for file in info["siblings"]
            .as_array()
            .context("Dataset files missing")?
        {
            let name = file["rfilename"].as_str().context("Dataset filename")?;
            let rel = relative(name)?;
            if ignored(rel) {
                continue;
            }
            let mut url =
                reqwest::Url::parse(&format!("{HOST}/datasets/{repo}/resolve/{revision}/"))?;
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Hub URL"))?
                .pop_if_empty()
                .extend(name.split('/'));
            let dest = stage.0.join(rel);
            fs::create_dir_all(dest.parent().unwrap())?;
            let mut response = check(auth(client.get(url), &token).send()?)?;
            let mut file = File::create_new(&dest)?;
            std::io::copy(&mut response, &mut file)?;
            file.sync_all()?;
        }
        ensure!(
            stage.0.join("meta/info.json").is_file(),
            "Downloaded repository is not a LeRobot dataset"
        );
        // A failed transfer never exposes a partial dataset or blocks a retry.
        // Directory rename can replace an empty destination but cannot overwrite
        // a nonempty folder created by someone else while downloading.
        fs::rename(&stage.0, path)?;
        return Ok(());
    }
    ensure!(mode == "push", "Unknown Hub operation");
    ensure!(
        token.is_some(),
        "Log in to Hugging Face or set HF_TOKEN before uploading."
    );
    ensure!(
        path.join("meta/info.json").is_file(),
        "Open a LeRobot dataset first"
    );
    let create = auth(client.post(format!("{HOST}/api/repos/create")), &token)
        .json(&json!({"name":name,"organization":owner,"type":"dataset","private":true}))
        .send()?;
    if create.status() != reqwest::StatusCode::CONFLICT {
        check(create)?;
    }
    let mut paths = Vec::new();
    files(path, path, &mut paths)?;
    paths.sort();
    let mut operations = Vec::new();
    operations.push(json!({"key":"header","value":{"summary":"Upload dataset from DoRobot Studio","description":""}}));
    for rel in paths {
        let full = path.join(&rel);
        let name = rel.to_str().context("UTF-8 dataset path required")?;
        let (size, oid, sample) = digest(&full)?;
        let pre: Value = check(
            auth(
                client.post(format!("{HOST}/api/datasets/{repo}/preupload/main")),
                &token,
            )
            .json(&json!({"files":[{"path":name,"sample":sample,"size":size}]}))
            .send()?,
        )?
        .json()?;
        let entry = &pre["files"][0];
        ensure!(entry["path"] == name, "Hub preupload path mismatch");
        if entry["shouldIgnore"].as_bool() == Some(true) {
            continue;
        }
        if entry["uploadMode"] == "regular" || size == 0 {
            ensure!(
                size <= 16 * 1024 * 1024,
                "Hub requested an oversized regular file"
            );
            operations.push(json!({"key":"file","value":{"path":name,"encoding":"base64","content":B64.encode(fs::read(&full)?)}}));
        } else {
            ensure!(entry["uploadMode"] == "lfs", "Unsupported Hub upload mode");
            let batch:Value=check(auth(client.post(format!("{HOST}/datasets/{repo}.git/info/lfs/objects/batch")),&token).header("Content-Type","application/vnd.git-lfs+json").json(&json!({"operation":"upload","transfers":["basic"],"objects":[{"oid":oid,"size":size}],"hash_algo":"sha256","ref":{"name":"main"}})).send()?)?.json()?;
            let object = &batch["objects"][0];
            ensure!(
                object["error"].is_null() && object["oid"] == oid,
                "Hub LFS upload refused"
            );
            if !object["actions"]["upload"].is_null() {
                let action = &object["actions"]["upload"];
                let href = action["href"].as_str().context("LFS upload URL")?;
                ensure!(href.starts_with("https://"), "Insecure LFS upload URL");
                let mut upload = client.put(href).header("Content-Length", size);
                if let Some(headers) = action["header"].as_object() {
                    for (k, v) in headers {
                        upload = upload.header(k, v.as_str().context("LFS header")?);
                    }
                }
                // Use only the storage action's headers; never forward the HF bearer token.
                check(upload.body(Body::new(File::open(&full)?)).send()?)?;
                if let Some(verify) = object["actions"]["verify"]["href"].as_str() {
                    let url = reqwest::Url::parse(verify)?;
                    ensure!(
                        url.scheme() == "https" && url.host_str() == Some("huggingface.co"),
                        "Unexpected LFS verify origin"
                    );
                    check(
                        auth(client.post(url), &token)
                            .json(&json!({"oid":oid,"size":size}))
                            .send()?,
                    )?;
                }
            }
            operations.push(json!({"key":"lfsFile","value":{"path":name,"algo":"sha256","oid":oid,"size":size}}));
        }
    }
    let mut body = Vec::new();
    for operation in operations {
        serde_json::to_writer(&mut body, &operation)?;
        body.write_all(b"\n")?;
    }
    check(
        auth(
            client.post(format!("{HOST}/api/datasets/{repo}/commit/main")),
            &token,
        )
        .header("Content-Type", "application/x-ndjson")
        .body(body)
        .send()?,
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_remote_path_escape() {
        for p in ["/tmp/x", "../x", "a/../../x", "."] {
            assert!(relative(p).is_err());
        }
        assert!(relative("videos/chunk-000/cam/episode.mp4").is_ok());
    }
    #[test]
    fn validate_repository_before_io() {
        for p in [
            "oops",
            "org/../repo",
            "org/a/b",
            "org/.private",
            "org/name.git",
        ] {
            assert!(repo_id(p).is_err());
        }
        assert!(repo_id("dorobot/nano-demo").is_ok());
    }
}
